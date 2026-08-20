# Production-source macro audit — RELEASE_CHECKLIST.md:44

Pin: `eb473293ecc5847986f30af5d3b773fdbb1dcab6` (beamr 0.19.4)
Venue: MacBookPro / Darwin 25.3.0 arm64
Date: 2026-08-20

Selector: `eprintln!` · `dbg!` · `todo!(` · `unimplemented!(` · `panic!(`
Scope: `crates/*/src/**/*.rs` (345 files)

## Verdict

**PASS.** Five production-source hits, all deliberate and self-documenting.
Zero stray debug output. The 0.19.4 delta adds none.

## Why the raw number is not the finding

A bare grep over `crates/*/src` returns **447 hits**. That number is about test
code, not production code: beamr keeps unit tests *inside* `src/`, in files
wired up as `#[cfg(test)] #[path = "X_tests.rs"] mod tests;`.

Three successive classifier bugs, each of which made the result look cleaner or
dirtier than it was — recorded because the shape recurs:

1. In-file rule matched only `tests.rs`, missing all `*_tests.rs`. → 214 false
   "production" hits.
2. Gating test matched `cfg(test)` and `cfg(any(test`, but **not
   `cfg(all(test, feature = "net"))`** → 3 files reported UNGATED that are gated.
3. Parent-module search keyed on the *file stem*, but these modules are declared
   as `mod tests;` with a `#[path]` attribute → 20 files with "no declaration
   found". Same trap in the 2018-style `foo.rs` + `foo/` layout, where the
   parent dir is not the module dir.

Each bug is a **dead selector**, and a dead selector reads exactly like a clean
result. Resolved: 63 `#[cfg(test)]`-gated test files, **0 ungated `*_tests.rs`
module declarations**.

447 raw → 12 after gating → **5 after reading the survivors**.

## The five, each read rather than counted

| Site | Verdict |
|---|---|
| `crates/beamr-cli/src/main.rs:463,475,481` | CLI warnings to stderr (`beamr: warning: skipped …`). Legitimate: this is a binary whose job includes telling the operator what it skipped. |
| `crates/beamr/src/capability/audit.rs:55` | Inside `StderrViolationHandler`, a **named, opt-in** `ViolationHandler` impl documented as "writes denied capability context to stderr". The write *is* the type's purpose; a caller opts in by installing it. |
| `crates/beamr/src/interpreter/mod.rs:274` | Gated behind `BEAMR_TRACE_IP`, read once into a `OnceLock<bool>`, off by default, comment names the switch. Deliberate opt-in VM tracing. |

## Delta check

`c5d05af` (0.19.3) → `eb47329` (0.19.4), `crates/` only:

```
 crates/beamr/Cargo.toml          |  2 +-
 crates/beamr/src/jit/profiler.rs | 27 +++++++++++++++++++++++----
```

Lines added by the delta matching the selector: **none**.
(Selector positive-controlled against a synthetic `+ eprintln!("x");` line: matches.)

## Instrument controls run

- `rustfmt --check` on deliberately misformatted input → **rc=1**, so the `fmt`
  leg's 0-byte log is *clean*, not *asleep*.
- Classifier positive controls: a known test hit, a known `#[cfg(test)] mod`
  hit, and a known production hit each classified as expected before the
  residue was believed.
