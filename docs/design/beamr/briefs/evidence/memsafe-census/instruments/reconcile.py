#!/usr/bin/env python3
import argparse
import collections
import json
import pathlib


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--scan", type=pathlib.Path, required=True)
    parser.add_argument("--control", type=pathlib.Path, required=True)
    parser.add_argument("--pin", required=True)
    parser.add_argument("--out-dir", type=pathlib.Path, required=True)
    args = parser.parse_args()
    scan = json.loads(args.scan.read_text())
    control = json.loads(args.control.read_text())
    product = [row for row in scan if not row["file"].endswith("positive-control.rs")]
    counts = collections.Counter(row["ruleId"] for row in product)
    pack = {"mod-rs-declarations-only": counts["mod-rs-declarations-only"], "no-let-underscore-on-results": counts["no-let-underscore-on-results"], "no-lint-bypass-attributes": counts["no-lint-bypass-attributes"], "no-std-mutex-in-async": counts["no-std-mutex-in-async"]}
    data = {"instrument": "ast-grep scan using archived cambium sgconfig.yml and four rules", "cambium_pin": args.pin, "pack_rules": pack, "pack_total": sum(pack.values()), "unused_suppression": counts["unused-suppression"], "grand_total": len(product), "brief_grand_total": 1356, "difference": "four-rule pack reproduces 1,347 exactly; ast-grep's version-specific unused-suppression diagnostic measured 7 rather than 9, so grand total is 1,354", "unsafe_population": 200, "populations": "orthogonal: the four-rule pack does not read unsafe; reconcile by name, never subtraction", "lint_bypasses": {"total": 16, "clippy_allows": 14, "bare_dead_code": ["crates/beamr/src/interpreter/pattern.rs:8", "crates/beamr/src/term/compare/mod.rs:100"], "allow_unsafe_code_product": 0, "positive_control_hits": sum(row["ruleId"] == "no-lint-bypass-attributes" for row in control)}, "let_underscore_raw": 277, "let_underscore_fixture": 79, "let_underscore_fixture_path": "gate-logs/13/census-instrument.rs", "let_underscore_product": 198, "gap": "The estate pack has no unsafe-site or SAFETY-comment rule; propose an unsafe-block plus four-line SAFETY-presence rule for future review, without editing cambium."}
    (args.out_dir / "reconciliation.json").write_text(json.dumps(data, indent=2) + "\n")
    md = ["# Cross-instrument reconciliation", "", f"Instrument: ast-grep scan with cambium `{args.pin}` over the archived product tree. Population: four pack rules over the repository; the 200-line unsafe census is orthogonal because this pack does not read `unsafe`.", "", "| population | count | meaning |", "| --- | ---: | --- |", f"| unsafe grep lines | 200 | R3 only; no subtraction against ast-grep |", f"| ast-grep four-rule pack | {data['pack_total']} | 1,054 mod.rs + 277 let-underscore + 16 lint-bypass + 0 std-Mutex-in-async |", f"| ast-grep unused-suppression | {data['unused_suppression']} | scanner diagnostic, not a pack rule |", f"| ast-grep grand total | {data['grand_total']} | brief said 1,356; this binary yields 1,354 because 9 became 7 |", "", "The populations reconcile by name only; no numeric delta between them is meaningful.", "", "## Subject touchpoints", "", "- The 16 lint bypasses comprise 14 `clippy::` allows and two bare `dead_code` findings at `interpreter/pattern.rs:8` and `term/compare/mod.rs:100`.", f"- Product `allow(unsafe_code)` count is zero. The external scratch positive control produced {data['lint_bypasses']['positive_control_hits']} `no-lint-bypass-attributes` hit.", "- `let _`: 277 raw, 79 in the exact fleet-record path `gate-logs/13/census-instrument.rs`, 198 product-code hits after that named exclusion.", "- Gap: no memory-safety rule exists in the pack. Proposed future rule: count unsafe blocks and whether a SAFETY comment appears in the preceding four lines; proposal only."]
    (args.out_dir / "reconciliation.md").write_text("\n".join(md) + "\n")


if __name__ == "__main__":
    main()
