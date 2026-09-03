#!/bin/bash
# cprime.sh — supplementary axis-C pass: probe only, --dir = DIRS.txt MINUS the erts ebin (which carries erlang.beam,
# erts_internal.beam, init.beam, prim_*.beam: bytecode stand-ins for BIF/NIF modules that mask missing natives).
ROOT=/home/aion/reach/elixir-reach-20260903; PROBE=$ROOT/beamr/target/debug/examples/elixir_reach_probe; OTP=/usr/local/lib/otp-29.0.5/lib/erlang/lib
echo "cprime start $(date -u +%FT%TZ)"
ls $OTP/erts-*/ebin/erlang.beam $OTP/erts-*/ebin/erts_internal.beam $OTP/erts-*/ebin/prim_file.beam || { echo "ABORT: expected preloaded beams absent"; exit 7; }
echo "erts ebin members: $(ls $OTP/erts-*/ebin/*.beam | wc -l): $(ls $OTP/erts-*/ebin/*.beam | xargs -n1 basename | sed 's/\.beam//' | tr '\n' ' ')"
grep -v "/erts-" $ROOT/DIRS.txt > $ROOT/DIRS-noerts.txt; echo "dirs=$(wc -l < $ROOT/DIRS-noerts.txt)"
DIRS=$(sed 's/^/--dir /' $ROOT/DIRS-noerts.txt | tr '\n' ' ')
run() { t=$1; l=$2; mkdir -p $ROOT/outC/$t; while read -r beam; do [ -n "$beam" ] || continue; m=$(basename "$beam" .beam); $PROBE "$beam" $DIRS > $ROOT/outC/$t/$m.probe.json 2> $ROOT/outC/$t/$m.probe.err; echo $? > $ROOT/outC/$t/$m.probe.rc; done < "$l"; echo "$t done $(date -u +%FT%TZ)"; }
split -n l/2 -d $ROOT/lists/T2.txt $ROOT/lists/T2.cpart
run T2 $ROOT/lists/T2.cpart00 & run T2 $ROOT/lists/T2.cpart01 & run T3 $ROOT/lists/T3.txt & run T4 $ROOT/lists/T4.txt & run T1 $ROOT/lists/T1.txt & run CTRL $ROOT/lists/CTRL.txt &
wait
echo "cprime done $(date -u +%FT%TZ)"; touch $ROOT/cprime.done
