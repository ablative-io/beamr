#!/bin/bash
ROOT=/home/aion/reach/elixir-reach-20260903; T=$ROOT/tools
echo "tiers start $(date -u +%FT%TZ)"
for t in T2 T3 T4; do bash $T/run-tier.sh $t $ROOT/lists/$t.txt 2>&1 | tail -1; done
python3 $T/status.py $ROOT/out $ROOT/sets/NATIVE-MFAS.txt
echo "tiers done $(date -u +%FT%TZ)"; touch $ROOT/tiers.done
