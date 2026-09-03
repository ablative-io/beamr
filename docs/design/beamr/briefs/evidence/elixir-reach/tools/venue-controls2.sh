#!/bin/bash
set -uo pipefail
ROOT=/home/aion/reach/elixir-reach-20260903; T=$ROOT/tools; B=$ROOT/beamr; export PATH=$HOME/.cargo/bin:$PATH
LANE=dc26cec76e21ebc13b3769a5d19097918d89650e
cd $B && git fetch -q https://github.com/ablative-io/beamr.git refs/heads/elixir-reach-20260903:refs/remotes/lane/elixir-reach-20260903 && git checkout -q --detach $LANE && echo "beamr lane head=$(git rev-parse HEAD)"
CARGO_BUILD_JOBS=6 cargo build -p beamr --example elixir_reach_probe 2>&1 | tail -1
PROBE=$B/target/debug/examples/elixir_reach_probe; CLI=$B/target/debug/beamr
echo "== native census from the registry (venue) =="; $PROBE --natives > $ROOT/sets/NATIVE-MFAS.txt; echo "rc=$? mfas=$(wc -l < $ROOT/sets/NATIVE-MFAS.txt) modules=$(cut -d: -f1 $ROOT/sets/NATIVE-MFAS.txt | sort -u | wc -l)"
echo "== CTRL tier through the driver =="; bash $T/run-tier.sh CTRL $ROOT/lists/CTRL.txt 2>&1 | tail -12
echo "== control 2: wport9 runnable seven under the beamr CLI =="
cd $B; for F in wake_send wake_cast wake_receive_timeout wake_timer_deadline bif_supported bif_unsupported output_entry; do out=$(timeout 120 $CLI conformance/workload/wport9_conformance.beam --entry wport9_conformance:$F/0 --dir conformance/workload 2>&1); rc=$?; echo "entry=$F rc=$rc last=$(printf '%s' "$out" | tail -c 160 | tr '\n' ' ')"; done
echo "-- process_error/0 (recorded, not asserted)"; out=$(timeout 120 $CLI conformance/workload/wport9_conformance.beam --entry wport9_conformance:process_error/0 --dir conformance/workload 2>&1); echo "rc=$? last=$(printf '%s' "$out" | tail -c 300 | tr '\n' ' ')"
echo "== control 4/5: line_coverage (183, expect rc 0) and beam_debug_info (184, expect UnknownOpcode 184) =="
for m in reach_plain reach_linecov reach_debuginfo; do out=$(timeout 60 $CLI $ROOT/controls/$m.beam --entry reach_lc:main/0 2>&1); echo "$m rc=$? out=$(printf '%s' "$out" | tail -c 240 | tr '\n' ' ')"; done
echo "== control 3: CLI imports on op191 / op200 (expect non-zero quoting unsupported opcode N) =="
for m in reach_op191 reach_op200; do out=$($CLI imports $ROOT/controls191/$m.beam 2>&1); echo "$m rc=$? out=$(printf '%s' "$out" | tail -c 200 | tr '\n' ' ')"; done
echo "== control 6: reach_zzz imports =="; $CLI imports $ROOT/controls/reach_zzz.beam; echo "rc=$?"
echo "== per-control merged summary =="; for f in $ROOT/out/CTRL/*.json; do python3 -c "
import json,sys; j=json.load(open('$f')); C=j['C']; g=j['A']['generic']
print(j['module'], j['load_status'], j['first_refused_opcode'], 'gen=',g['set_tuple_element']['count'],g['executable_line']['count'],g['debug_line']['count'], 'agree=',j['A']['agreement'][:30], 'C=', (str(len(C['unresolved']))+'u/'+str(C['deferred'])+'d/'+str(len(C['denied']))+'x eq='+str(C['cli_stream_equal'])) if isinstance(C,dict) else C, 'unres=', [u['mfa']+'x'+str(u['calls']) for u in C['unresolved']][:4] if isinstance(C,dict) else '')"; done
echo "done=$(date -u +%FT%TZ)"
