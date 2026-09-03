#!/bin/bash
set -euo pipefail
ROOT=/home/aion/reach/elixir-reach-20260903; T=$ROOT/tools; C=$ROOT/controls; OTP=/usr/local/lib/otp-29.0.5/lib/erlang/lib
cd "$C"
echo "== compile controls =="
rm -f *.beam *.S.orig
erlc reach_lc.erl && mv reach_lc.beam reach_plain.beam
erlc +line_coverage -o . reach_lc.erl && mv reach_lc.beam reach_linecov.beam
erlc +beam_debug_info -o . reach_lc.erl && mv reach_lc.beam reach_debuginfo.beam
erlc reach_zzz.erl
erlc -S reach_lc.erl && mv reach_lc.S reach_lc.S.orig
ls -la *.beam *.S.orig
echo "== instrument on wport9 (full to wport9.otp.tsv) =="; escript $T/reach_otp.escript $ROOT/beamr/conformance/workload/wport9_conformance.beam > $C/wport9.otp.tsv; grep -E "^(module|walk|callext)" $C/wport9.otp.tsv; grep -cE "^op\s" $C/wport9.otp.tsv
echo "== instrument on the three controls =="
for f in reach_plain reach_linecov reach_debuginfo reach_zzz; do echo "-- $f"; escript $T/reach_otp.escript $f.beam | grep -E "^(module|walk|op\s+(67|183|184)\s|callext)" ; done
echo "== instrument sweep over stdlib+kernel ebin (self-test: every module walks ok, max seen <= header) =="
n=0; bad=0; for f in $OTP/stdlib-*/ebin/*.beam $OTP/kernel-*/ebin/*.beam; do n=$((n+1)); w=$(escript $T/reach_otp.escript "$f" | awk -F'\t' '$1=="walk"{print $2}'); [ "$w" = "ok" ] || { bad=$((bad+1)); echo "NOT OK: $f -> $w"; }; done; echo "swept=$n not_ok=$bad"
echo "== supervisor.beam (control 1 OTP side) =="; escript $T/reach_otp.escript $OTP/stdlib-*/ebin/supervisor.beam | grep -E "^(walk|impt)" | head -30
echo "== max opcode across sweep =="; for f in $OTP/stdlib-*/ebin/*.beam $OTP/kernel-*/ebin/*.beam; do escript $T/reach_otp.escript "$f" | awk -F'\t' '$1=="op"{print $2}'; done | sort -n | uniq -c | sort -k2 -n | tail -8
