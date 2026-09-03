#!/bin/bash
set -euo pipefail
ROOT=/home/aion/reach/elixir-reach-20260903; EX=$ROOT/corpus/elixir-1.20.4; OTP=/usr/local/lib/otp-29.0.5/lib/erlang/lib
mkdir -p $ROOT/sets $ROOT/lists
{ ls -d $EX/lib/*/ebin; ls -d $OTP/*/ebin; } > $ROOT/DIRS.txt; echo "DIRS=$(wc -l < $ROOT/DIRS.txt)"
ls $ROOT/t1/*.beam > $ROOT/lists/T1.txt
ls $EX/lib/elixir/ebin/*.beam > $ROOT/lists/T2.txt
tail -n +2 $ROOT/T3-REACH.tsv | cut -f4 > $ROOT/lists/T3.txt
{ ls $EX/lib/iex/ebin/*.beam $EX/lib/mix/ebin/*.beam; cut -f3 $ROOT/t4-otp-beyond-t3.tsv; } > $ROOT/lists/T4.txt
ls $ROOT/controls/*.beam $ROOT/controls191/reach_op191.beam $ROOT/controls191/reach_op200.beam $OTP/stdlib-*/ebin/supervisor.beam $ROOT/beamr/conformance/workload/wport9_conformance.beam > $ROOT/lists/CTRL.txt
wc -l $ROOT/lists/*.txt
echo "== MANIFEST.tsv =="
{ echo -e "tier\tpath\tsha256\tbytes"; for t in T1 T2 T3 T4 CTRL; do while read -r f; do echo -e "$t\t$f\t$(sha256sum "$f" | cut -d' ' -f1)\t$(stat -c %s "$f")"; done < $ROOT/lists/$t.txt; done; } > $ROOT/MANIFEST.tsv
echo "manifest rows=$(($(wc -l < $ROOT/MANIFEST.tsv)-1))"; sha256sum $ROOT/corpus/elixir-otp-29.zip
