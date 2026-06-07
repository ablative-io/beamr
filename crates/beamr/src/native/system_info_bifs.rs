//! `erlang:system_info/1` VM introspection BIFs.
//!
//! The supported item set is deliberately small and demand-driven by OTP
//! compatibility needs. Unsupported items raise `badarg` rather than exposing a
//! larger Erlang/OTP runtime configuration surface.

use crate::atom::{Atom, AtomTable};
use crate::native::{BifRegistryImpl, Capability, NativeRegistrationError, ProcessContext};
use crate::term::Term;

const PROCESS_LIMIT: usize = 262_144;
const ATOM_LIMIT: usize = u32::MAX as usize + 1;
const WORDSIZE_BYTES: i64 = 8;
const OTP_RELEASE: &[u8] = b"27";

/// Read-only VM metrics used by `system_info/1`.
pub trait SystemInfoFacility: Send + Sync {
    /// Number of live processes in the process table.
    fn process_count(&self) -> usize;

    /// Number of normal scheduler threads configured for the VM.
    fn scheduler_count(&self) -> usize;

    /// Number of currently interned atoms.
    fn atom_count(&self) -> usize;

    /// Maximum number of live processes supported by the VM.
    fn process_limit(&self) -> usize {
        PROCESS_LIMIT
    }
}

/// Registers `erlang:system_info/1`.
pub fn register_system_info_bifs(
    registry: &BifRegistryImpl,
    atom_table: &AtomTable,
) -> Result<(), NativeRegistrationError> {
    let erlang = atom_table.intern("erlang");
    let system_info_atom = atom_table.intern("system_info");

    registry.register(erlang, system_info_atom, 1, system_info, Capability::Pure)
}

/// `erlang:system_info/1` for the OTP-library subset beamr supports.
pub fn system_info(args: &[Term], context: &mut ProcessContext) -> Result<Term, Term> {
    let [item] = args else {
        return Err(badarg());
    };
    let atom = item.as_atom().ok_or_else(badarg)?;
    let atom_table = context.atom_table().ok_or_else(badarg)?;
    let name = atom_table.resolve(atom).ok_or_else(badarg)?;
    let metrics = metrics(context)?;

    match name {
        "schedulers" => integer_term(metrics.scheduler_count()),
        "process_count" => integer_term(metrics.process_count()),
        "process_limit" => integer_term(metrics.process_limit()),
        "wordsize" => Ok(Term::small_int(WORDSIZE_BYTES)),
        "otp_release" => context.alloc_binary(OTP_RELEASE),
        "version" => context.alloc_binary(env!("CARGO_PKG_VERSION").as_bytes()),
        "system_architecture" => context.alloc_binary(system_architecture().as_bytes()),
        "atom_count" => integer_term(metrics.atom_count()),
        "atom_limit" => integer_term(ATOM_LIMIT),
        _ => Err(badarg()),
    }
}

fn metrics<'context>(
    context: &'context ProcessContext<'_>,
) -> Result<&'context dyn SystemInfoFacility, Term> {
    context.system_info_facility().ok_or_else(badarg)
}

fn integer_term(value: usize) -> Result<Term, Term> {
    let value = i64::try_from(value).map_err(|_| badarg())?;
    Term::try_small_int(value).ok_or_else(badarg)
}

fn system_architecture() -> String {
    option_env!("TARGET").map_or_else(
        || format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS),
        str::to_owned,
    )
}

fn badarg() -> Term {
    Term::atom(Atom::BADARG)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::process::Process;
    use crate::term::binary::Binary;

    use super::*;

    struct TestSystemInfoFacility {
        process_count: usize,
        scheduler_count: usize,
        atom_count: usize,
    }

    impl SystemInfoFacility for TestSystemInfoFacility {
        fn process_count(&self) -> usize {
            self.process_count
        }

        fn scheduler_count(&self) -> usize {
            self.scheduler_count
        }

        fn atom_count(&self) -> usize {
            self.atom_count
        }
    }

    fn call_system_info(item_name: &str) -> Result<Term, Term> {
        let atom_table = Arc::new(AtomTable::with_common_atoms());
        let item = Term::atom(atom_table.intern(item_name));
        let facility = Arc::new(TestSystemInfoFacility {
            process_count: 3,
            scheduler_count: 5,
            atom_count: atom_table.len(),
        });
        let mut process = Process::new(0, 256);
        let mut context = ProcessContext::new();
        context.set_atom_table(Some(atom_table));
        context.set_system_info_facility(Some(facility));
        context.attach_process(&mut process, 1);

        system_info(&[item], &mut context)
    }

    fn binary_bytes(term: Term) -> Result<Vec<u8>, String> {
        Binary::new(term)
            .map(|binary| binary.as_bytes().to_vec())
            .ok_or_else(|| "expected binary term".to_owned())
    }

    fn binary_string(term: Term) -> Result<String, String> {
        String::from_utf8(binary_bytes(term)?).map_err(|error| error.to_string())
    }

    #[test]
    fn supported_integer_items_return_expected_values() -> Result<(), String> {
        let schedulers = call_system_info("schedulers").map_err(|term| format!("{term:?}"))?;
        let process_count =
            call_system_info("process_count").map_err(|term| format!("{term:?}"))?;
        let process_limit =
            call_system_info("process_limit").map_err(|term| format!("{term:?}"))?;
        let wordsize = call_system_info("wordsize").map_err(|term| format!("{term:?}"))?;
        let atom_count = call_system_info("atom_count").map_err(|term| format!("{term:?}"))?;
        let atom_limit = call_system_info("atom_limit").map_err(|term| format!("{term:?}"))?;

        assert_eq!(schedulers.as_small_int(), Some(5));
        assert_eq!(process_count.as_small_int(), Some(3));
        assert_eq!(process_limit.as_small_int(), Some(PROCESS_LIMIT as i64));
        assert_eq!(wordsize.as_small_int(), Some(WORDSIZE_BYTES));
        assert!(atom_count.as_small_int().is_some_and(|count| count > 0));
        assert_eq!(atom_limit.as_small_int(), Some(ATOM_LIMIT as i64));
        Ok(())
    }

    #[test]
    fn supported_binary_items_return_expected_values() -> Result<(), String> {
        let otp_release = call_system_info("otp_release").map_err(|term| format!("{term:?}"))?;
        let version = call_system_info("version").map_err(|term| format!("{term:?}"))?;
        let architecture =
            call_system_info("system_architecture").map_err(|term| format!("{term:?}"))?;

        assert_eq!(binary_bytes(otp_release)?, OTP_RELEASE);
        assert_eq!(binary_string(version)?, env!("CARGO_PKG_VERSION"));
        assert!(!binary_string(architecture)?.is_empty());
        Ok(())
    }

    #[test]
    fn unknown_non_atom_and_wrong_arity_return_badarg() {
        assert_eq!(call_system_info("unknown_item"), Err(badarg()));

        let mut context = ProcessContext::new();
        assert_eq!(
            system_info(&[Term::small_int(1)], &mut context),
            Err(badarg())
        );
        assert_eq!(system_info(&[], &mut context), Err(badarg()));
        assert_eq!(
            system_info(&[Term::small_int(1), Term::small_int(2)], &mut context),
            Err(badarg())
        );
    }

    #[test]
    fn missing_metrics_facility_returns_badarg_for_all_items() {
        let atom_table = Arc::new(AtomTable::with_common_atoms());
        let item = Term::atom(atom_table.intern("wordsize"));
        let mut process = Process::new(0, 256);
        let mut context = ProcessContext::new();
        context.set_atom_table(Some(atom_table));
        context.attach_process(&mut process, 1);

        assert_eq!(system_info(&[item], &mut context), Err(badarg()));
    }

    #[test]
    fn registration_installs_erlang_system_info_1() -> Result<(), String> {
        let atom_table = AtomTable::with_common_atoms();
        let registry = BifRegistryImpl::new();

        register_system_info_bifs(&registry, &atom_table).map_err(|error| error.to_string())?;

        let erlang = atom_table
            .lookup("erlang")
            .ok_or_else(|| "missing erlang atom".to_owned())?;
        let function = atom_table
            .lookup("system_info")
            .ok_or_else(|| "missing system_info atom".to_owned())?;
        let entry = registry
            .lookup(erlang, function, 1)
            .ok_or_else(|| "missing registered BIF".to_owned())?;

        assert_eq!(entry.capability, Capability::Pure);
        Ok(())
    }
}
