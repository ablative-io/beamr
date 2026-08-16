"""AR-1 gate rows 2 + R10 -- the ledger checker.

⭐ WITHOUT POSITIVE EVIDENCE AT THE SITE, "structurally eliminated" IS AN
UNFALSIFIABLE CLAIM IN A TABLE. This script is the difference between a
disposition that is machine-verified and one that is asserted, which is the
gate's amendment-2 condition 2.

Two modes:

  (default)     PRE-FIX. `PENDING` rows are legal; everything else is checked.
  --sign-off    every crossing must carry a real disposition. ⛔ SILENCE ABOUT
                A SITE IS THE FAILURE: a count falling to zero with seventeen
                named dispositions is a PASS, falling to zero with seventeen
                silences is the defect row 2 exists to catch.

  --self-test   builds deliberately-broken ledgers and requires each check to
                reject the right one for the right reason. ⭐ A CHECK THAT HAS
                NEVER BEEN SHOWN TO REFUSE IS NOT A CHECK. The
                STRUCTURALLY-ELIMINATED arm has ZERO rows to act on today, so
                without the self-test it would ship as prose in a script's
                clothing -- green forever, having never run.

Exit 0 = every check that could fire, fired clean. Exit 2 = refusal.
"""
import argparse
import copy
import json
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
LEDGER = HERE / "dispositions.json"
# The repo root: .../beamr/docs/design/beamr/briefs/evidence/accumulator-rooting
REPO = HERE.parents[5]

CROSSING_DISPOSITIONS = {"SAFE-ROOTED", "STRUCTURALLY-ELIMINATED",
                         "FIXED-UNVERIFIED", "PENDING"}
REQUIRED_CROSSING = ("id", "file", "function", "carrier", "reason",
                     "disposition", "replacement_construct", "replacement_site",
                     "verification_leg")
REQUIRED_FIXTURE = ("id", "file", "function", "marker", "shape_class", "reason",
                    "disposition", "excluded_from_remediation",
                    "exclusion_basis", "recheck_trigger")


def check(ledger, repo, sign_off):
    """Returns (failures, fired) -- fired names the checks that actually ran."""
    failures, fired = [], []
    sites = ledger["sites"]

    crossings = [s for s in sites if s.get("disposition") != "CONTROL-FIXTURE"]
    fixtures = [s for s in sites if s.get("disposition") == "CONTROL-FIXTURE"]

    # -- 1. population -------------------------------------------------------
    fired.append("population")
    declared = ledger["population"]["count"]
    if len(crossings) != declared:
        failures.append(f"population: {len(crossings)} crossing rows, "
                        f"declared {declared}")
    ids = [s["id"] for s in crossings]
    if sorted(ids) != list(range(1, declared + 1)):
        failures.append(f"population: crossing ids are not exactly 1..{declared}: "
                        f"{sorted(ids, key=str)}")
    all_ids = [s["id"] for s in sites]
    if len(set(all_ids)) != len(all_ids):
        dupes = sorted({i for i in all_ids if all_ids.count(i) > 1}, key=str)
        failures.append(f"population: duplicate ids {dupes}")

    # -- 2. schema + disposition vocabulary ---------------------------------
    fired.append("schema")
    for s in crossings:
        missing = [f for f in REQUIRED_CROSSING if f not in s]
        if missing:
            failures.append(f"site {s.get('id')}: missing fields {missing}")
        if s.get("disposition") not in CROSSING_DISPOSITIONS:
            failures.append(f"site {s.get('id')}: disposition "
                            f"{s.get('disposition')!r} is not one of "
                            f"{sorted(CROSSING_DISPOSITIONS)}")
    for s in fixtures:
        missing = [f for f in REQUIRED_FIXTURE if f not in s]
        if missing:
            failures.append(f"fixture {s.get('id')}: missing fields {missing}")

    # -- 3. every cited file exists -----------------------------------------
    fired.append("file-presence")
    for s in sites:
        if not (repo / s["file"]).is_file():
            failures.append(f"site {s['id']}: cited file {s['file']} does not exist")

    # -- 4. fixture markers are PRESENT AT THE SITE -------------------------
    # The exclusion is by exact literal path AND exact binding name, so both
    # halves are checked. A path that exists proves nothing about the marker.
    fired.append("fixture-markers")
    for s in fixtures:
        p = repo / s["file"]
        if not p.is_file():
            continue
        body = p.read_text()
        n = body.count(s["marker"])
        if n == 0:
            failures.append(f"fixture {s['id']}: marker {s['marker']!r} absent from "
                            f"{s['file']} -- the control it names cannot fire")
        elif n < 2:
            # binding + at least one use; a marker that appears once is a
            # dangling name, which means the fixture body has been gutted.
            failures.append(f"fixture {s['id']}: marker {s['marker']!r} appears once "
                            f"in {s['file']} -- bound but never used, fixture gutted?")

    # -- 5. STRUCTURALLY-ELIMINATED carries positive evidence AT the site ----
    elim = [s for s in crossings if s["disposition"] == "STRUCTURALLY-ELIMINATED"]
    if elim:
        fired.append("structural-evidence")
    for s in elim:
        construct, site = s.get("replacement_construct"), s.get("replacement_site")
        if not construct or not site:
            failures.append(f"site {s['id']}: STRUCTURALLY-ELIMINATED without "
                            f"replacement_construct and replacement_site -- an "
                            f"unfalsifiable claim in a table")
            continue
        try:
            path, lineno = site.rsplit(":", 1)
            lineno = int(lineno)
        except ValueError:
            failures.append(f"site {s['id']}: replacement_site {site!r} is not file:line")
            continue
        p = repo / path
        if not p.is_file():
            failures.append(f"site {s['id']}: replacement_site file {path} does not exist")
            continue
        lines = p.read_text().splitlines()
        if not 1 <= lineno <= len(lines):
            failures.append(f"site {s['id']}: replacement_site line {lineno} is outside "
                            f"{path} ({len(lines)} lines)")
            continue
        if construct not in lines[lineno - 1]:
            failures.append(f"site {s['id']}: replacement construct {construct!r} is NOT "
                            f"present at {site} -- the claim is refuted at the bytes")

    # -- 6. sign-off refuses silence ----------------------------------------
    if sign_off:
        fired.append("sign-off-silence")
        pending = [s["id"] for s in crossings if s["disposition"] == "PENDING"]
        if pending:
            failures.append(f"SIGN-OFF: {len(pending)} crossing(s) still PENDING: "
                            f"{pending}. Silence about a site is the failure")
        unverified = [s["id"] for s in crossings
                      if s["disposition"] == "FIXED-UNVERIFIED"]
        if unverified:
            failures.append(f"SIGN-OFF: {len(unverified)} crossing(s) FIXED-UNVERIFIED: "
                            f"{unverified}. Sign-blocking by rule, never silent")
    return failures, fired


def self_test(ledger, repo):
    """Every check above must be shown to REFUSE, on this run."""
    cases = []

    def case(name, mutate, expect):
        m = copy.deepcopy(ledger)
        mutate(m)
        cases.append((name, m, expect))

    # ⭐ THE GUINEA PIG IS SYNTHETIC NOW, AND THAT IS THE WHOLE POINT.
    #
    # Three arms below used to borrow site 4, because site 4 was the last
    # PENDING row in the ledger. #115 discharged it, and those three arms went
    # quiet in the same commit -- a self-test CONSUMED BY ITS OWN LANE
    # SUCCEEDING. It did not rot and it did not break; it ran out of defect to
    # feed on, and reported 7/10 while the checker itself was perfectly fine.
    #
    # ⛔ A CONTROL THAT ONLY WORKS WHILE A DEFECT IS OUTSTANDING IS NOT A
    # CONTROL. It reads as strongest exactly when there is least to prove, and
    # falls silent at the moment its verdict starts to matter -- which is the
    # moment someone signs off on a fully-dispositioned ledger. The rows below
    # are MINTED HERE, so every arm stays falsifiable no matter what the real
    # ledger says, today or after the next lane empties it further.
    # The population check requires the crossing ids to be exactly 1..N, so the
    # minted row takes the next id rather than a memorable sentinel.
    SYNTHETIC_ID = 1 + max(s["id"] for s in ledger["sites"]
                           if isinstance(s.get("id"), int))
    ANCHOR_FILE = "crates/beamr/src/native/dictionary_bifs.rs"
    ANCHOR_TOKEN = "dict_entries_to_list"

    # The anchor line is FOUND, never counted to. The previous arm hard-coded
    # line 128 and silently degraded to an empty string when the file reflowed,
    # which is how a passing arm turns into a failing one without anybody
    # touching the checker.
    anchor_lines = (repo / ANCHOR_FILE).read_text().splitlines()
    anchor_hits = [n for n, text in enumerate(anchor_lines, 1)
                   if ANCHOR_TOKEN in text]
    if not anchor_hits:
        raise SystemExit(
            f"SELF-TEST CANNOT RUN: anchor token {ANCHOR_TOKEN!r} is absent from "
            f"{ANCHOR_FILE}. Re-point ANCHOR_FILE/ANCHOR_TOKEN at any line that "
            f"really exists -- do NOT weaken the arms to make this pass.")
    ANCHOR_LINE = anchor_hits[0]
    print(f"  anchor: {ANCHOR_FILE}:{ANCHOR_LINE} carries {ANCHOR_TOKEN!r}")

    def add_synthetic(m, **overrides):
        """Append a minted crossing row, keeping the declared population honest."""
        row = {"id": SYNTHETIC_ID, "file": ANCHOR_FILE,
               "function": "self_test_guinea_pig", "carrier": "synthetic",
               "reason": "minted by self_test; never a real crossing",
               "disposition": "PENDING", "replacement_construct": None,
               "replacement_site": None, "verification_leg": "native"}
        row.update(overrides)
        m["sites"].append(row)
        m["population"]["count"] += 1

    def drop_a_crossing(m):
        m["sites"] = [s for s in m["sites"] if s.get("id") != 5]
    case("population/short", drop_a_crossing, "population")

    def dupe_id(m):
        for s in m["sites"]:
            if s.get("id") == 6:
                s["id"] = 5
    case("population/duplicate-id", dupe_id, "duplicate ids")

    def bad_disposition(m):
        for s in m["sites"]:
            if s.get("id") == 1:
                s["disposition"] = "PROBABLY-FINE"
    case("schema/vocabulary", bad_disposition, "is not one of")

    def missing_field(m):
        for s in m["sites"]:
            if s.get("id") == 2:
                del s["carrier"]
    case("schema/missing-field", missing_field, "missing fields")

    def bad_file(m):
        for s in m["sites"]:
            if s.get("id") == 3:
                s["file"] = "crates/beamr/src/does_not_exist.rs"
    case("file-presence", bad_file, "does not exist")

    def bad_marker(m):
        for s in m["sites"]:
            if s.get("id") == "CF-S3c":
                s["marker"] = "s3c_pair_renamed_by_a_refactor"
    case("fixture-markers", bad_marker, "absent from")

    # The STRUCTURALLY-ELIMINATED arm is fired in BOTH directions -- a claim
    # that is true at the bytes must pass, and one that is false must be
    # refused. One arm alone would be satisfied by a checker that always says
    # yes, or always says no.
    def elim_unevidenced(m):
        add_synthetic(m, disposition="STRUCTURALLY-ELIMINATED")
    case("structural/no-evidence", elim_unevidenced, "unfalsifiable claim")

    def elim_refuted(m):
        add_synthetic(m, disposition="STRUCTURALLY-ELIMINATED",
                      replacement_construct="with_rooted_prereserved_NOT_PRESENT",
                      replacement_site=f"{ANCHOR_FILE}:{ANCHOR_LINE}")
    case("structural/refuted-at-bytes", elim_refuted, "NOT present at")

    def elim_true(m):
        # A construct that IS present at that line, read from the file.
        add_synthetic(m, disposition="STRUCTURALLY-ELIMINATED",
                      replacement_construct=ANCHOR_TOKEN,
                      replacement_site=f"{ANCHOR_FILE}:{ANCHOR_LINE}")
    case("structural/true-must-PASS", elim_true, None)

    # Sign-off's refusal is fired on a MINTED PENDING row too. Reading it off
    # the live ledger meant the arm passed only while the ledger was still
    # incomplete -- the same parasitism, in the one check whose whole purpose is
    # to stand between an incomplete ledger and a signature.
    def sign_off_silence(m):
        add_synthetic(m, disposition="PENDING")
    pending_case = copy.deepcopy(ledger)
    sign_off_silence(pending_case)

    ok = True
    for name, m, expect in cases:
        failures, _ = check(m, repo, sign_off=False)
        if expect is None:
            hit = not failures
            print(f"  {'PASS' if hit else 'FAIL'}  {name:32s} "
                  f"{'accepted a true claim' if hit else failures}")
        else:
            hit = any(expect in f for f in failures)
            print(f"  {'PASS' if hit else 'FAIL'}  {name:32s} "
                  f"{'refused: ' + expect if hit else 'DID NOT REFUSE -- ' + str(failures)}")
        ok &= hit

    # sign-off silence, on a ledger carrying one minted PENDING row
    failures, _ = check(pending_case, repo, sign_off=True)
    refusal = next((f for f in failures if "still PENDING" in f), None)
    print(f"  {'PASS' if refusal else 'FAIL'}  {'sign-off/silence':32s} "
          f"{'refused: ' + refusal if refusal else 'DID NOT REFUSE'}")

    # ⚠️ AND THE NEGATIVE HALF, which the borrowed-row version could never
    # have: the SAME ledger without the minted row must NOT be refused. A
    # sign-off check that refuses everything would satisfy the arm above.
    clean, _ = check(copy.deepcopy(ledger), repo, sign_off=True)
    silent = not any("still PENDING" in f for f in clean)
    print(f"  {'PASS' if silent else 'FAIL'}  {'sign-off/not-over-eager':32s} "
          f"{'accepted a fully-dispositioned ledger' if silent else clean}")
    return ok and bool(refusal) and silent


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--sign-off", action="store_true")
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args()

    ledger = json.loads(LEDGER.read_text())

    if args.self_test:
        print("SELF-TEST -- every check must be shown to refuse, this run:")
        ok = self_test(ledger, REPO)
        print(f"\n{'✅ every check fired' if ok else '🔴 a check could not be made to refuse'}")
        return 0 if ok else 2

    failures, fired = check(ledger, REPO, args.sign_off)
    mode = "SIGN-OFF" if args.sign_off else "PRE-FIX"
    print(f"AR-1 disposition ledger -- {mode}")
    print(f"  {len(ledger['sites'])} rows: "
          f"{sum(1 for s in ledger['sites'] if s.get('disposition') != 'CONTROL-FIXTURE')} "
          f"crossings + "
          f"{sum(1 for s in ledger['sites'] if s.get('disposition') == 'CONTROL-FIXTURE')} "
          f"control fixtures")
    print(f"  checks fired: {', '.join(fired)}")
    unfired = [c for c in ("structural-evidence",) if c not in fired]
    if unfired:
        print(f"  ⚠️ checks that could NOT fire (no rows to act on): "
              f"{', '.join(unfired)} -- run --self-test, which forces them")
    if failures:
        print("\n⛔ REFUSED:")
        for f in failures:
            print(f"  {f}")
        return 2
    print("\n✅ ledger consistent.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
