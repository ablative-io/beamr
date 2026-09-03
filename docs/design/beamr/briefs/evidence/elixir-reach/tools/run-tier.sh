#!/bin/bash
# run-tier.sh <tier> <list-of-beams-file>  — on 205. Per module: probe JSON, walker TSV, CLI imports, merge.
set -uo pipefail
ROOT=/home/aion/reach/elixir-reach-20260903; T=$ROOT/tools; OUT=$ROOT/out; TIER=$1; LIST=$2
PROBE=$ROOT/beamr/target/debug/examples/elixir_reach_probe; CLI=$ROOT/beamr/target/debug/beamr
DIRS=$(cat $ROOT/DIRS.txt | sed 's/^/--dir /' | tr '\n' ' ')
mkdir -p $OUT/$TIER/raw
n=0; while read -r beam; do [ -n "$beam" ] || continue; n=$((n+1)); m=$(basename "$beam" .beam)
  $PROBE "$beam" $DIRS > $OUT/$TIER/raw/$m.probe.json 2> $OUT/$TIER/raw/$m.probe.err; echo $? > $OUT/$TIER/raw/$m.probe.rc
  escript $T/reach_otp.escript "$beam" > $OUT/$TIER/raw/$m.otp.tsv 2>&1
  $CLI imports "$beam" $DIRS > $OUT/$TIER/raw/$m.cli.txt 2> $OUT/$TIER/raw/$m.cli.err; echo $? > $OUT/$TIER/raw/$m.cli.rc
  python3 $T/merge.py $TIER "$beam" $OUT/$TIER/raw/$m.probe.json $OUT/$TIER/raw/$m.otp.tsv $OUT/$TIER/raw/$m.cli.txt "$(cat $OUT/$TIER/raw/$m.cli.rc)" $ROOT/sets/DECODER-SET.txt $ROOT/sets/EXECUTED-SET.txt $ROOT/T3-REACH.tsv $OUT/$TIER/$m.json || echo "MERGE FAILED $TIER $m"
done < "$LIST" | tee $OUT/$TIER.log
echo "tier=$TIER modules=$(wc -l < $OUT/$TIER.log) done=$(date -u +%FT%TZ)"
