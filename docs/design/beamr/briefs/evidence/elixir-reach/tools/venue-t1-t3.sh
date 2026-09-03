#!/bin/bash
set -euo pipefail
ROOT=/home/aion/reach/elixir-reach-20260903; T=$ROOT/tools; OTP=/usr/local/lib/otp-29.0.5/lib/erlang/lib; EX=$ROOT/corpus/elixir-1.20.4
echo "== T1 compile with the zip's elixirc =="
cd $ROOT/t1; rm -f *.beam
export PATH=$EX/bin:$PATH
elixir --version 2>&1 | tail -2
for f in t1_*.ex; do elixirc --ignore-module-conflict -o . "$f"; done
ls -la *.beam
echo "== OTP module index (path-defined) =="
ls $OTP/*/ebin/*.beam | awk -F/ '{p=$0; app=$(NF-2); m=$NF; sub(/\.beam$/,"",m); print m"\t"app"\t"p}' | sort > $ROOT/otp-modules.tsv
wc -l $ROOT/otp-modules.tsv
echo "== T3 scan: ImpT of every T2 module =="
escript $T/impt.escript $EX/lib/elixir/ebin/*.beam > $ROOT/t2-impt.tsv
echo "t2 files=$(ls $EX/lib/elixir/ebin/*.beam | wc -l) impt rows=$(wc -l < $ROOT/t2-impt.tsv)"
cut -f2 $ROOT/t2-impt.tsv | sort -u > $ROOT/t2-imported-modules.txt
echo "distinct imported modules=$(wc -l < $ROOT/t2-imported-modules.txt)"
# depth 1: imported modules that are OTP modules
join -t $'\t' <(sort $ROOT/t2-imported-modules.txt) <(cut -f1,2,3 $ROOT/otp-modules.tsv) > $ROOT/t3-depth1.tsv
echo "depth1 OTP modules=$(wc -l < $ROOT/t3-depth1.tsv)"
# depth 2: ImpT of depth-1 modules, OTP only, not already at depth 1
cut -f3 $ROOT/t3-depth1.tsv | xargs escript $T/impt.escript > $ROOT/t3-depth1-impt.tsv
cut -f2 $ROOT/t3-depth1-impt.tsv | sort -u > $ROOT/t3-d1-imported.txt
join -t $'\t' <(sort $ROOT/t3-d1-imported.txt) <(cut -f1,2,3 $ROOT/otp-modules.tsv) | join -t $'\t' -v1 - <(cut -f1 $ROOT/t3-depth1.tsv | sort) > $ROOT/t3-depth2.tsv
echo "depth2 OTP modules (new)=$(wc -l < $ROOT/t3-depth2.tsv)"
# always-included
ALWAYS="gen_server supervisor gen proc_lib ets persistent_term code code_server unicode unicode_util application application_controller $(ls $OTP/kernel-*/ebin/logger*.beam | xargs -n1 basename | sed 's/\.beam$//' | tr '\n' ' ')"
{ echo -e "module\tapp\tdepth\tpath"
  awk -F'\t' '{print $1"\t"$2"\t1\t"$3}' $ROOT/t3-depth1.tsv
  awk -F'\t' '{print $1"\t"$2"\t2\t"$3}' $ROOT/t3-depth2.tsv
  for m in $ALWAYS; do grep -P "^$m\t" $ROOT/otp-modules.tsv | awk -F'\t' '{print $1"\t"$2"\talways\t"$3}'; done
} | awk -F'\t' 'NR==1{print;next} !seen[$1]++' > $ROOT/T3-REACH.tsv
echo "T3-REACH rows=$(($(wc -l < $ROOT/T3-REACH.tsv)-1)) by depth: $(tail -n +2 $ROOT/T3-REACH.tsv | cut -f3 | sort | uniq -c | tr '\n' ' ')"
echo "apps reached: $(tail -n +2 $ROOT/T3-REACH.tsv | cut -f2 | sort | uniq -c | sort -rn | head -12 | tr '\n' ' ')"
echo "== T4 corpus: iex + mix ebin, plus their OTP reach beyond T3 =="
echo "iex=$(ls $EX/lib/iex/ebin/*.beam | wc -l) mix=$(ls $EX/lib/mix/ebin/*.beam | wc -l)"
escript $T/impt.escript $EX/lib/iex/ebin/*.beam $EX/lib/mix/ebin/*.beam | cut -f2 | sort -u > $ROOT/t4-imported.txt
join -t $'\t' $ROOT/t4-imported.txt <(cut -f1,2,3 $ROOT/otp-modules.tsv) | join -t $'\t' -v1 - <(tail -n +2 $ROOT/T3-REACH.tsv | cut -f1 | sort) > $ROOT/t4-otp-beyond-t3.tsv
echo "T4 OTP modules beyond T3=$(wc -l < $ROOT/t4-otp-beyond-t3.tsv): $(cut -f1 $ROOT/t4-otp-beyond-t3.tsv | tr '\n' ' ')"
echo "== non-OTP, non-Elixir imported modules referenced by T2 (should be empty or named) =="
comm -23 $ROOT/t2-imported-modules.txt <(cut -f1 $ROOT/otp-modules.tsv | sort) | grep -v '^Elixir\.' | grep -v -x -F -f <(ls $EX/lib/*/ebin/*.beam | xargs -n1 basename | sed 's/\.beam$//' | sort) || true
echo "done=$(date -u +%FT%TZ)"
