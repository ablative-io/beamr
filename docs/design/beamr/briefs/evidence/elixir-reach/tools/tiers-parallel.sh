#!/bin/bash
ROOT=/home/aion/reach/elixir-reach-20260903; T=$ROOT/tools
echo "tiers-parallel start $(date -u +%FT%TZ)"
# resume-aware: skip modules whose merged JSON already exists
todo() { while read -r b; do m=$(basename "$b" .beam); [ -s "$ROOT/out/$1/$m.json" ] || echo "$b"; done < "$ROOT/lists/$1.txt"; }
todo T2 > $ROOT/lists/T2.todo; todo T3 > $ROOT/lists/T3.todo; todo T4 > $ROOT/lists/T4.todo
split -n l/2 -d $ROOT/lists/T2.todo $ROOT/lists/T2.part
echo "todo: T2=$(wc -l < $ROOT/lists/T2.todo) T3=$(wc -l < $ROOT/lists/T3.todo) T4=$(wc -l < $ROOT/lists/T4.todo)"
bash $T/run-tier.sh T2 $ROOT/lists/T2.part00 .a > $ROOT/tiers.T2a.log 2>&1 &
bash $T/run-tier.sh T2 $ROOT/lists/T2.part01 .b > $ROOT/tiers.T2b.log 2>&1 &
bash $T/run-tier.sh T3 $ROOT/lists/T3.todo > $ROOT/tiers.T3.log 2>&1 &
bash $T/run-tier.sh T4 $ROOT/lists/T4.todo > $ROOT/tiers.T4.log 2>&1 &
wait
python3 $T/status.py $ROOT/out $ROOT/sets/NATIVE-MFAS.txt
for t in T2 T3 T4; do echo "$t merged=$(ls $ROOT/out/$t/*.json 2>/dev/null | wc -l) of $(wc -l < $ROOT/lists/$t.txt)"; done
echo "tiers-parallel done $(date -u +%FT%TZ)"; touch $ROOT/tiers.done
