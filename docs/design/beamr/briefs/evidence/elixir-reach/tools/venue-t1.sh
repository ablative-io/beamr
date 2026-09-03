#!/bin/bash
set -uo pipefail
ROOT=/home/aion/reach/elixir-reach-20260903; T=$ROOT/tools; B=$ROOT/beamr; CLI=$B/target/debug/beamr; OTP=/usr/local/lib/otp-29.0.5/lib/erlang/lib
echo "== control 4/5 again WITH --dir stdlib ebin (so lists:sum/1 can resolve as bytecode) =="
for m in reach_plain reach_linecov reach_debuginfo; do out=$(timeout 60 $CLI $ROOT/controls/$m.beam --entry reach_lc:main/0 --dir $OTP/stdlib-8.0.3/ebin 2>&1); echo "$m rc=$? out=$(printf '%s' "$out" | grep -v '^beamr: warning' | tail -c 240 | tr '\n' ' ') skipped_line=$(printf '%s' "$out" | grep -c 'warning: skipped') "; done
echo "-- does lists.beam itself load? "; $B/target/debug/examples/elixir_reach_probe $OTP/stdlib-8.0.3/ebin/lists.beam | python3 -c "import json,sys; j=json.load(sys.stdin); print('lists.beam', j['load_status'], j['first_refused_opcode'], j['load_error'])"
echo "== rerun CTRL with the merge columns =="; bash $T/run-tier.sh CTRL $ROOT/lists/CTRL.txt 2>&1 | tail -1
echo "== T1 =="; bash $T/run-tier.sh T1 $ROOT/lists/T1.txt 2>&1
for f in $ROOT/out/T1/*.json; do python3 -c "
import json; j=json.load(open('$f')); C=j['C']; g=j['A']['generic']
print(j['module'], j['load_status'], j['first_refused_opcode'], 'gen=',g['set_tuple_element']['count'],g['executable_line']['count'],g['debug_line']['count'], 'skipped=',j['skipped_in_dirs'], 'C=', (str(len(C['unresolved']))+'u/'+str(C['deferred'])+'d/'+str(len(C['denied']))+'x eq='+str(C['cli_stream_equal'])) if isinstance(C,dict) else C, [u['mfa'] for u in C['unresolved']][:6] if isinstance(C,dict) else '', 'D=',len(j['D']) if isinstance(j['D'],list) else j['D'])"; done
echo "== T1 run under the CLI (Elixir.T1Hello main/0 etc., with all DIRS) =="
DIRS=$(sed 's/^/--dir /' $ROOT/DIRS.txt | tr '\n' ' ')
for m in T1Hello T1EnumMap T1String T1Struct T1Process T1Task; do out=$(timeout 120 $CLI $ROOT/t1/Elixir.$m.beam --entry Elixir.$m:main/0 $DIRS 2>&1); rc=$?; echo "$m rc=$rc out=$(printf '%s' "$out" | grep -v '^beamr: warning' | tail -c 200 | tr '\n' ' ')"; done
echo "done=$(date -u +%FT%TZ)"
