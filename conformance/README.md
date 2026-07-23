# WPORT-9 conformance gate

The permanent browser/Worker conformance harness (brief
`docs/design/beamr/briefs/WPORT-9.json`): one driverless driver, three real
artifacts, a committed bytecode workload, and the F-0d timer-shim windows
that make NO-POLLING a machine-checked gate.

## Run

```sh
node conformance/driver.mjs
```

Exit 0 iff every leg passes; the output ends with a per-leg named verdict
list (those names are CI carriers — additions and removals move the
workflow's carrier list same-commit).

Prerequisites, all FAIL-LOUD (a missing tool errors by name; no leg can
skip): `wasm-pack`, the Rust `wasm32-unknown-unknown` target, Node ≥ 20,
and a Chrome binary — `CHROME_BIN` if set, else the macOS default path.

## Workload

`workload/wport9_conformance.erl` (compiled `.beam` committed beside it —
erlc from OTP 29; recompile with `erlc wport9_conformance.erl` in that
directory). One exported entry per acceptance-shape surface; entries
terminate in maps the driver machine-checks. Wake classes unreachable from
bytecode (trapped exit, native completion) are ledger rows in the arc doc
with their T1 walls named — see the brief's tier table.

## Release-record binding (D4c, decided text)

Release declarations MUST cite the workflow's green conformance-job
(browser-leg) run at the release tree. The gates battery's `wasm-tests`
leg carries the Node tier on every box; the browser tier is
permanent-workflow territory and never lands in `gates.json`. On a box
without Chrome this driver ERRORS, never skips — a release cannot fake
the browser leg.
