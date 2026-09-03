#!/bin/bash
set -euo pipefail
ROOT=/home/aion/reach/elixir-reach-20260903; T=$ROOT/tools; OTP=/usr/local/lib/otp-29.0.5/lib/erlang/lib
echo "== sweep ALL OTP ebin (self-test: every module walks ok; opcode max seen vs header) =="
n=0; bad=0; : > $ROOT/sweep-maxop.txt
for f in $OTP/*/ebin/*.beam; do n=$((n+1)); out=$(escript $T/reach_otp.escript "$f" 2>&1); w=$(printf '%s' "$out" | awk -F'\t' '$1=="walk"{print $2}'); hdr=$(printf '%s' "$out" | awk -F'\t' '$1=="opcode_max_header"{print $2}'); mx=$(printf '%s' "$out" | awk -F'\t' '$1=="op"{print $2}' | sort -n | tail -1); echo "$f	$w	$hdr	$mx" >> $ROOT/sweep-maxop.txt; [ "$w" = "ok" ] || { bad=$((bad+1)); echo "NOT OK: $f -> $w"; }; done
echo "swept=$n not_ok=$bad"
echo "header==max_seen: $(awk -F'\t' '$3==$4' $ROOT/sweep-maxop.txt | wc -l)  header!=max_seen: $(awk -F'\t' '$3!=$4' $ROOT/sweep-maxop.txt | wc -l)"
echo "modules whose max opcode > 184 (beamr ceiling): $(awk -F'\t' '$4>184' $ROOT/sweep-maxop.txt | wc -l) of $n"
echo "distinct opcodes > 184 across OTP: $(for f in $OTP/*/ebin/*.beam; do escript $T/reach_otp.escript "$f" | awk -F'\t' '$1=="op" && $2>184 {print $2}'; done | sort -n | uniq -c | tr '\n' ' ')"
echo "== T2 sweep (Elixir core, 271) =="
EX=$ROOT/corpus/elixir-1.20.4; n=0; bad=0; for f in $EX/lib/elixir/ebin/*.beam; do n=$((n+1)); w=$(escript $T/reach_otp.escript "$f" | awk -F'\t' '$1=="walk"{print $2}'); [ "$w" = "ok" ] || { bad=$((bad+1)); echo "NOT OK: $f -> $w"; }; done; echo "t2 swept=$n not_ok=$bad"
echo "distinct opcodes > 184 across T2: $(for f in $EX/lib/elixir/ebin/*.beam; do escript $T/reach_otp.escript "$f" | awk -F'\t' '$1=="op" && $2>184 {print $2}'; done | sort -n | uniq -c | tr '\n' ' ')"
echo "T2 modules with any opcode > 184: $(for f in $EX/lib/elixir/ebin/*.beam; do escript $T/reach_otp.escript "$f" | awk -F'\t' -v f="$f" '$1=="op" && $2>184 {print f; exit}'; done | wc -l)"
echo "T2 modules with 183 or 184: $(for f in $EX/lib/elixir/ebin/*.beam; do escript $T/reach_otp.escript "$f" | awk -F'\t' -v f="$f" '$1=="op" && ($2==183||$2==184) {print f; exit}'; done | wc -l)"
echo "done=$(date -u +%FT%TZ)"
