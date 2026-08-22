//! Code management native facilities and BIFs.
//!
//! These BIFs expose the scheduler code-server API to BEAM code without
//! duplicating hot-load or purge logic in native functions.

use crate::atom::{Atom, AtomTable};
use crate::error::LoadError;
use crate::module::{ModuleOrigin, PurgeError};
use crate::native::{
    BifRegistryImpl, Capability, NativeFn, NativeRegistrationError, ProcessContext,
};
use crate::scheduler::{HotLoadResult, PurgeResult};
use crate::term::Term;
use crate::term::binary_ref::BinaryRef;

/// Scheduler-backed code management operations used by hot-code BIFs.
pub trait CodeManagementFacility: Send + Sync {
    /// Load raw BEAM bytes as a new module version.
    fn load_module(&self, bytes: &[u8]) -> Result<HotLoadResult, LoadError>;

    /// Attempt to safely purge retained old code.
    fn purge_module(&self, module: Atom) -> Result<PurgeResult, PurgeError>;

    /// Remove all versions of a module from the registry.
    fn delete_module(&self, module: Atom) -> bool;

    /// Return true when retained old code exists for `module`.
    fn check_old_code(&self, module: Atom) -> bool;

    /// Return true when `pid` is running or pinned to old code for `module`.
    fn check_process_code(&self, pid: u64, module: Atom) -> bool;

    /// Return origin metadata for a current loaded module.
    fn module_origin(&self, module: Atom) -> Option<ModuleOrigin>;

    /// Return all currently loaded module names and origins.
    fn all_loaded_modules(&self) -> Vec<(Atom, ModuleOrigin)>;
}

type CodeBif = (&'static str, u8, Capability, NativeFn);

const CODE_BIFS: &[CodeBif] = &[
    ("load_module", 2, Capability::ExternalIo, load_module),
    ("purge_module", 1, Capability::ExternalIo, purge_module),
    ("delete_module", 1, Capability::ExternalIo, delete_module),
    ("check_old_code", 1, Capability::ExternalIo, check_old_code),
    (
        "check_process_code",
        2,
        Capability::ExternalIo,
        check_process_code,
    ),
];

/// Registers code-management BIFs under the `erlang` module.
pub fn register_code_management_bifs(
    registry: &BifRegistryImpl,
    atom_table: &AtomTable,
) -> Result<(), NativeRegistrationError> {
    let erlang = atom_table.intern("erlang");
    for &(function_name, arity, capability, native_function) in CODE_BIFS {
        let function = atom_table.intern(function_name);
        registry.register(erlang, function, arity, native_function, capability)?;
    }
    let code = atom_table.intern("code");
    let all_loaded_name = atom_table.intern("all_loaded");
    registry.register(code, all_loaded_name, 0, all_loaded, Capability::Pure)?;
    Ok(())
}

/// erlang:load_module/2. The first argument names the module; the second is
/// BEAM bytes.
pub fn load_module(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let [name_term, bytes_term] = args else {
        return Err(badarg());
    };
    let module_name = name_term.as_atom().ok_or_else(badarg)?;
    let bytes = BinaryRef::new(*bytes_term)
        .ok_or_else(badarg)?
        .as_bytes(context.borrow_terms())
        .to_vec();
    let facility = context.code_management_facility().ok_or_else(badarg)?;
    let result = facility.load_module(&bytes).map_err(|_| badarg())?;
    if result.module_name != module_name {
        return Err(badarg());
    }
    context.alloc_tuple(&[Term::atom(Atom::MODULE), Term::atom(result.module_name)])
}

/// erlang:purge_module/1.
pub fn purge_module(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let [module_term] = args else {
        return Err(badarg());
    };
    let module = module_term.as_atom().ok_or_else(badarg)?;
    let facility = context.code_management_facility().ok_or_else(badarg)?;
    facility.purge_module(module).map_err(|_| badarg())?;
    Ok(bool_term(true))
}

/// erlang:delete_module/1.
pub fn delete_module(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let [module_term] = args else {
        return Err(badarg());
    };
    let module = module_term.as_atom().ok_or_else(badarg)?;
    let facility = context.code_management_facility().ok_or_else(badarg)?;
    Ok(bool_term(facility.delete_module(module)))
}

/// erlang:check_old_code/1.
pub fn check_old_code(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let [module_term] = args else {
        return Err(badarg());
    };
    let module = module_term.as_atom().ok_or_else(badarg)?;
    let facility = context.code_management_facility().ok_or_else(badarg)?;
    Ok(bool_term(facility.check_old_code(module)))
}

/// erlang:check_process_code/2.
pub fn check_process_code(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let [pid_term, module_term] = args else {
        return Err(badarg());
    };
    let pid = pid_term.as_pid().ok_or_else(badarg)?;
    let module = module_term.as_atom().ok_or_else(badarg)?;
    let facility = context.code_management_facility().ok_or_else(badarg)?;
    Ok(bool_term(facility.check_process_code(pid, module)))
}

/// code:all_loaded/0 returns currently loaded modules with their source metadata.
pub fn all_loaded(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    if !args.is_empty() {
        return Err(badarg());
    }
    let facility = context.code_management_facility().ok_or_else(badarg)?;
    let loaded = facility.all_loaded_modules();
    let atom_table = context.atom_table().ok_or_else(badarg)?;
    let loaded_terms: Vec<(Term, Term)> = loaded
        .into_iter()
        .map(|(module, origin)| {
            let source = atom_table.intern(origin.source_atom_name());
            (Term::atom(module), Term::atom(source))
        })
        .collect();

    // AR-1 site 3. The carrier used to be a threaded `list` tail consed up in
    // reverse — a boxed cons held across `alloc_tuple`, which collects. The
    // accumulator holds every entry in the process root stack instead, so a
    // collection mid-loop forwards them; `to_list` then hands the whole run to
    // `alloc_list`, which roots its own arguments.
    context.with_accumulator(|context, entries| {
        for (module, source) in loaded_terms {
            let tuple = context.alloc_tuple(&[module, source])?;
            entries.push(context, tuple)?;
        }
        entries.to_list(context)
    })
}

fn bool_term(value: bool) -> Term {
    Term::atom(if value { Atom::TRUE } else { Atom::FALSE })
}

fn badarg() -> Term {
    Term::atom(Atom::BADARG)
}

#[cfg(test)]
mod ar1_row4_site3_tests {
    // ✅ INVERTED — these now assert CORRECT BEHAVIOUR. AR-1 site 3 is FIXED.
    //
    // Until the fix lane these were DEFECT-ASSERTING: they pinned the measured
    // corrupt surface at f993280 (sweep A 3 red / 5 clean, sweep B 2 red / 7
    // clean) and were green because the defect was still present. The fix moved
    // that surface to ZERO on both axes, so the assertions are inverted rather
    // than deleted — the same cells, the opposite expectation.
    //
    // ⛔⛔ AND THAT INVERSION KILLED THE PROBE'S OWN POSITIVE CONTROL. The old
    // `a_red > 0` was what proved the sweep applied heap pressure at all. Post
    // fix it cannot hold, and a bare "0 corruption" is indistinguishable from a
    // sweep that has quietly stopped applying pressure — the two produce
    // identical output and mean opposite things.
    //
    // ⇒ `all_loaded_unrooted_replica` below is the replacement control: the
    // PRE-FIX BODY, kept verbatim, driven through the REAL allocator under the
    // SAME pressure regime, and asserted STILL TO CORRUPT. If it ever goes
    // quiet, the regime is gone and every green in this module is worthless.
    // Same law as the R10 control fixtures one level down — a control keyed on
    // a live defect is destroyed by the repair it exists to survive.

    use std::sync::Arc;

    use super::{CodeManagementFacility, all_loaded};
    use crate::atom::{Atom, AtomTable};
    use crate::error::LoadError;
    use crate::module::{ModuleOrigin, PurgeError};
    use crate::native::ProcessContext;
    use crate::process::Process;
    use crate::scheduler::{HotLoadResult, PurgeResult};
    use crate::term::Term;
    use crate::term::boxed::{Cons, Tuple};

    /// Facility stub whose only real method is `all_loaded_modules`. Every other
    /// method is refused rather than faked: this probe drives exactly one BIF,
    /// and a stub that answers questions nobody asked is a stub that can drift
    /// away from the trait without anything noticing.
    struct LoadedModulesFacility {
        modules: Vec<(Atom, ModuleOrigin)>,
    }

    impl CodeManagementFacility for LoadedModulesFacility {
        fn load_module(&self, _bytes: &[u8]) -> Result<HotLoadResult, LoadError> {
            Err(LoadError::DecodeError("unused by the site-3 probe".into()))
        }

        fn purge_module(&self, module: Atom) -> Result<PurgeResult, PurgeError> {
            Err(PurgeError::NoOldVersion { module })
        }

        fn delete_module(&self, _module: Atom) -> bool {
            false
        }

        fn check_old_code(&self, _module: Atom) -> bool {
            false
        }

        fn check_process_code(&self, _pid: u64, _module: Atom) -> bool {
            false
        }

        fn module_origin(&self, _module: Atom) -> Option<ModuleOrigin> {
            None
        }

        fn all_loaded_modules(&self) -> Vec<(Atom, ModuleOrigin)> {
            self.modules.clone()
        }
    }

    /// Which body a cell drives.
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Arm {
        /// The shipped `all_loaded`, rooted through `TermAccumulator`.
        Fixed,
        /// The pre-fix body, kept verbatim as this probe's positive control.
        UnrootedReplica,
    }

    /// ⛔⛔ THE SYNTHETIC POSITIVE — `all_loaded`'s body EXACTLY AS IT WAS
    /// BEFORE THE FIX, and it must stay that way.
    ///
    /// `list` is a threaded cons tail held in an ordinary local across
    /// `alloc_tuple`, which collects. This is AR-1 site 3 verbatim: it was
    /// `shape_hunt.py`'s original known-positive control, which is precisely
    /// why that control had to be re-sited to `ar1_shape_control.rs` before
    /// this lane could repair it.
    ///
    /// It is here so the inverted assertions above have something that still
    /// goes red. ⛔ Do NOT "tidy" it onto the accumulator — that deletes the
    /// control and leaves the greens next to it meaning nothing.
    fn all_loaded_unrooted_replica(context: &mut ProcessContext) -> Result<Term, Term> {
        let badarg = || Term::atom(Atom::BADARG);
        let facility = context.code_management_facility().ok_or_else(badarg)?;
        let loaded = facility.all_loaded_modules();
        let atom_table = context.atom_table().ok_or_else(badarg)?;
        let loaded_terms: Vec<(Term, Term)> = loaded
            .into_iter()
            .map(|(module, origin)| {
                let source = atom_table.intern(origin.source_atom_name());
                (Term::atom(module), Term::atom(source))
            })
            .collect();

        let mut list = Term::NIL;
        for (module, source) in loaded_terms.into_iter().rev() {
            let tuple = context.alloc_tuple(&[module, source])?;
            list = context.alloc_cons(tuple, list)?;
        }
        Ok(list)
    }

    /// One cell. Returns `(achieved_margin, outcome)`.
    ///
    /// `margin == None` means "no pre-fill at all" — the pure LENGTH axis. That
    /// distinction has to be explicit, because a pre-fill loop that happens to
    /// exit immediately is NOT the same experiment as one that never ran, and
    /// reporting them with the same number would collapse two states the output
    /// exists to separate.
    fn all_loaded_round_trip(
        modules: usize,
        heap: usize,
        margin: Option<usize>,
        arm: Arm,
    ) -> (usize, Result<(), String>) {
        let table = Arc::new(AtomTable::with_common_atoms());

        // Intern OUTSIDE the measured region. Atom interning touches the atom
        // table, not the process heap, but doing it up front keeps the heap
        // pressure attributable to `all_loaded` alone.
        let names: Vec<Atom> = (0..modules)
            .map(|index| table.intern(&format!("ar1_site3_module_{index:06}")))
            .collect();
        let facility = LoadedModulesFacility {
            modules: names
                .iter()
                .map(|module| (*module, ModuleOrigin::Preloaded))
                .collect(),
        };

        let mut process = Process::new(3, heap);
        let mut context = ProcessContext::new();
        context.set_atom_table(Some(Arc::clone(&table)));
        context.attach_process(&mut process, 0);
        context.set_code_management_facility(Some(Arc::new(facility)));

        // ⛔ THE PRE-FILL MUST BE ABLE TO GIVE UP, AND THE CELL MUST SAY SO.
        // Identical exit to sites 5 and 10, for the identical measured reason:
        // the descent step is one filler allocation (~6 words), so a requested
        // margin finer than that is only reachable via a collection, which frees
        // this unrooted filler and pushes `available` back up. The first version
        // of this loop elsewhere in the lane SPUN FOREVER. The achieved margin is
        // RETURNED so any give-up carries its own witness out with it.
        let mut filler = Vec::new();
        let achieved = if let Some(target) = margin {
            let mut last_available = usize::MAX;
            loop {
                let available = context.process_heap().map(|h| h.available()).unwrap_or(0);
                if available <= target || available >= last_available {
                    break available;
                }
                last_available = available;
                match context.alloc_binary(&[0x3C; 32]) {
                    Ok(term) => filler.push(term),
                    Err(_) => break available,
                }
            }
        } else {
            context.process_heap().map(|h| h.available()).unwrap_or(0)
        };

        let outcome = (|| -> Result<(), String> {
            let list = match arm {
                Arm::Fixed => all_loaded(&[], &mut context),
                Arm::UnrootedReplica => all_loaded_unrooted_replica(&mut context),
            }
            .map_err(|_| "all_loaded returned an error term".to_string())?;

            // The reader is ITERATIVE and HARD-CAPPED. A stale carrier can make a
            // cons tail alias an enclosing cell, turning the list into a CYCLE; a
            // recursive or uncapped walk hangs instead of reporting, and a hang
            // is the one failure this lane has already paid for once.
            let cap = modules * 2 + 16;
            let mut seen = 0usize;
            let mut tail = list;
            while !tail.is_nil() {
                if seen > cap {
                    return Err(format!(
                        "list did not terminate within {cap} cells — cyclic tail, carrier `list` went stale"
                    ));
                }
                let cons = Cons::new(tail).ok_or_else(|| {
                    format!("entry {seen}: tail is not a cons — carrier `list` went stale")
                })?;
                let tuple = Tuple::new(cons.head()).ok_or_else(|| {
                    format!("entry {seen}: head is not a tuple — carrier `list` went stale")
                })?;
                if tuple.arity() != 2 {
                    return Err(format!(
                        "entry {seen}: tuple arity {} not 2 — carrier `list` went stale",
                        tuple.arity()
                    ));
                }
                let module = tuple
                    .get(0)
                    .ok_or_else(|| format!("entry {seen}: no module slot"))?;
                // Compare by VALUE against the interned atom. Atoms are
                // immediates, so their contents cannot themselves go stale —
                // which is exactly why a mismatch here indicts the CARRIER and
                // nothing else.
                let want = *names
                    .get(seen)
                    .ok_or_else(|| format!("entry {seen}: more entries recovered than were put"))?;
                if module != Term::atom(want) {
                    return Err(format!(
                        "entry {seen}: module atom differs from the one put — carrier `list` went stale"
                    ));
                }
                seen += 1;
                tail = cons.tail();
            }
            if seen != modules {
                return Err(format!("recovered {seen} entries, put {modules}"));
            }
            Ok(())
        })();

        (achieved, outcome)
    }

    fn classify(cells: &[(String, usize, String)], label: &str, heap: usize) -> (usize, usize) {
        let corrupted = cells
            .iter()
            .filter(|(_, _, v)| v != "ok" && !v.contains("returned an error term"))
            .count();
        let clean = cells.iter().filter(|(_, _, v)| v == "ok").count();
        let refused = cells.len() - corrupted - clean;
        eprintln!(
            "site 3 {label}: {corrupted} corruption cells, {clean} clean, {refused} refused (heap {heap})"
        );
        for (axis, achieved, verdict) in cells {
            eprintln!("site 3 {label} {axis} achieved {achieved:>5} : {verdict}");
        }
        (corrupted, clean)
    }

    #[test]
    fn ar1_site3_all_loaded_band() {
        const HEAP: usize = 4096;

        // SWEEP A — LENGTH axis, no pre-fill. Spans the ~819 flip the pre-fix
        // arithmetic predicted, kept because that is where the defect used to
        // appear: the sweep must still visit the cells that once broke.
        const LENGTHS: [usize; 8] = [10, 50, 200, 500, 800, 1000, 1500, 2000];
        // SWEEP B — MARGIN axis, input pinned at 200 modules (1000 words needed,
        // which the empty 4096-word heap covers outright).
        const MARGINS: [usize; 9] = [2048, 1024, 512, 256, 128, 64, 32, 16, 8];

        let sweep = |arm: Arm| {
            let mut a = Vec::new();
            for modules in LENGTHS {
                let (achieved, result) = all_loaded_round_trip(modules, HEAP, None, arm);
                let verdict = match result {
                    Ok(()) => "ok".to_string(),
                    Err(reason) => reason,
                };
                a.push((format!("modules {modules:>5}"), achieved, verdict));
            }
            let mut b = Vec::new();
            for margin in MARGINS {
                let (achieved, result) = all_loaded_round_trip(200, HEAP, Some(margin), arm);
                let verdict = match result {
                    Ok(()) => "ok".to_string(),
                    Err(reason) => reason,
                };
                b.push((format!("margin req {margin:>5}"), achieved, verdict));
            }
            (a, b)
        };

        let (sweep_a, sweep_b) = sweep(Arm::Fixed);
        let (control_a, control_b) = sweep(Arm::UnrootedReplica);

        let (a_red, a_ok) = classify(&sweep_a, "FIXED sweep A (length)", HEAP);
        let (b_red, b_ok) = classify(&sweep_b, "FIXED sweep B (margin)", HEAP);
        let (ca_red, ca_ok) = classify(&control_a, "CONTROL sweep A (length)", HEAP);
        let (cb_red, cb_ok) = classify(&control_b, "CONTROL sweep B (margin)", HEAP);

        // Two-way controls, both required, per the site-10 law.
        // ⛔⛔ THE POSITIVE CONTROL COMES FIRST, and it is asserted BEFORE the
        // claim it licenses. The pre-fix body, same heap, same cells, must
        // still corrupt. If it does not, this sweep applies no usable pressure
        // and everything below is a green about nothing.
        assert!(
            ca_red > 0 && cb_red > 0,
            "POSITIVE CONTROL DEAD: the unrooted replica survived every cell on at least one \
             axis (A {ca_red} red / {ca_ok} clean, B {cb_red} red / {cb_ok} clean). The pressure \
             regime is gone, so site 3's zeros below mean nothing. Repair the regime — do NOT \
             weaken this assertion, and do NOT root the replica.\n\
             control A: {control_a:#?}\ncontrol B: {control_b:#?}"
        );
        assert!(
            ca_ok > 0,
            "NEGATIVE CONTROL DEAD: no replica LENGTH cell was clean, so the READER may be \
             broken rather than the carrier stale.\ncontrol A: {control_a:#?}"
        );

        // ⭐ THE CONTROL'S SURFACE IS THE ONE THAT WAS PINNED PRE-FIX, and it is
        // pinned UNCHANGED at (3, 5) and (2, 7) — the exact band measured at
        // f993280 against the shipped body. The replica reproduces it because it
        // IS that body. A drift here is a change in the allocator or the
        // collector, not in this lane.
        //
        // The sweep-B figure is NOT a target. It is the count of cells in which
        // the corruption happened to be VISIBLE — 2 of 9, against at least 6 in
        // which a collection fires mid-accumulation with a live carrier.
        assert_eq!(
            (ca_red, ca_ok, cb_red, cb_ok),
            (3, 5, 2, 7),
            "the unrooted replica's surface drifted from the band measured at f993280 against \
             the shipped body. The replica is supposed to BE that body.\n\
             control A (LENGTH, monotone, the real detector): {control_a:#?}\n\
             control B (MARGIN, deterministic but NON-MONOTONE): {control_b:#?}"
        );

        // ⛔ THE INTERPOLATION TRAP, ASSERTED SO IT CANNOT BE FORGOTTEN — and it
        // now lives on the CONTROL sweep, because non-monotonicity is a property
        // of the DEFECT's visibility and the fixed arm has no reds to be
        // non-monotone about. Moving it was forced by the inversion; dropping it
        // would have retired a finding rather than re-homing it.
        let first_red_b = control_b.iter().position(|(_, _, v)| v != "ok");
        let last_clean_b = control_b.iter().rposition(|(_, _, v)| v == "ok");
        assert!(
            matches!((first_red_b, last_clean_b), (Some(first), Some(last)) if last > first),
            "the MARGIN axis has become monotone (first red {first_red_b:?}, last clean \
             {last_clean_b:?}). The 'a clean margin cell is not evidence of safety' finding was \
             derived from NON-monotonicity — re-derive it, do not assume it.\n\
             control B: {control_b:#?}"
        );

        // ✅ THE CLAIM. Site 3 is rooted: ZERO corruption on either axis, every
        // cell clean, none refused — measured against a control that corrupted
        // five of the same cells in the same run.
        assert_eq!(
            (a_red, a_ok, b_red, b_ok),
            (0, LENGTHS.len(), 0, MARGINS.len()),
            "site 3 is NOT fully rooted: the accumulator arm still lost entries.\n\
             sweep A: {sweep_a:#?}\nsweep B: {sweep_b:#?}"
        );
    }
}
