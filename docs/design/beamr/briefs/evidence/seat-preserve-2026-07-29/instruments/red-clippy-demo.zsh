#!/bin/zsh
set -u
WT=/Users/annabel/Developer/ablative/stack/beamr/.wt-gates-929f4fc
EV=$WT/docs/design/beamr/briefs/evidence/r2gates-929f4fc/battery
CLAIM=/tmp/ablative-gate-battery.claim
SCRATCH=$WT/crates/beamr-cli/src/bin/scratch_red_negative_control.rs
cd "$WT" || exit 3
[[ -e $CLAIM ]] && { echo "HOLD: claim live"; cat "$CLAIM"; exit 42; }
OUT=$EV/red-demo-clippy-leg.txt
{
  echo "# R2 clippy-leg NEGATIVE CONTROL — separate result from the battery verdict"
  echo "# machine: Annabel's box; operator: Diana Plum (b337ce2b-336a-4856-a9d8-54c90496c9fb)"
  echo "# tree: $(git rev-parse --short HEAD) + ONE uncommitted scratch bin (deliberate unused_variables warning), removed after"
  echo "# purpose (RELEASE-0.17.0.md section 5): prove BOTH halves — (a) the leg FAILS, (b) the extract POPULATES"
  echo "# findings with the actual lint, file and line; plus confirm --keep-going still exits non-zero."
  echo "# load before:"; uptime
  echo "# start: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
} > "$OUT"
cat > "$SCRATCH" <<'RS'
fn main() {
    let unused_scratch_red_negative_control = 5;
}
RS
echo "# scratch file written: crates/beamr-cli/src/bin/scratch_red_negative_control.rs (uncommitted)" >> "$OUT"
CLIPPY_CMD="$(cat "$EV/clippy-leg-cmd.txt")"
echo "# cmd (verbatim from gates.json blob): $CLIPPY_CMD" >> "$OUT"
bash -c "$CLIPPY_CMD" \
  2>"$EV/red-demo-stderr.log" |
  tee "$EV/red-demo-json.log" |
  jq -c -f "$EV/clippy-extract.jq" > "$EV/red-demo-findings.jsonl"
RC=${pipestatus[1]}
{
  echo "# HALF (a): leg exit=$RC (non-zero REQUIRED; also confirms --keep-going preserves failure exit)"
  echo "# HALF (b): findings count = $(wc -l < "$EV/red-demo-findings.jsonl" | tr -d ' ')"
  echo "# findings lines mentioning the scratch lint:"
  grep "scratch_red_negative_control" "$EV/red-demo-findings.jsonl" | head -5
  echo "# end: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "# load after:"; uptime
} >> "$OUT"
rm "$SCRATCH"
echo "# scratch file removed; tracked-modified after: $(git status --porcelain | grep -cv '^??')" >> "$OUT"
echo "red-demo exit=$RC findings=$(wc -l < "$EV/red-demo-findings.jsonl" | tr -d ' ')"
tail -8 "$OUT"
