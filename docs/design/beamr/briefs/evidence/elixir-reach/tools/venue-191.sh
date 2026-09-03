#!/bin/bash
set -euo pipefail
ROOT=/home/aion/reach/elixir-reach-20260903; T=$ROOT/tools; D=$ROOT/controls191; mkdir -p $D; cd $D
cp $ROOT/controls/reach_lc.erl reach_op191.erl; sed -i 's/-module(reach_lc)\./-module(reach_op191)./' reach_op191.erl
erlc -S reach_op191.erl; cp reach_op191.S reach_op191.S.orig
echo "== .S original main/0 =="; awk '/^\{function, main, 0/,/^$/' reach_op191.S
# insert one get_record_field/5 (opcode 191 in OTP 29) right after the {line,...} that follows main/0's entry label
python3 - <<'PY'
import re
s=open('reach_op191.S').read()
m=re.search(r'(\{function, main, 0, \d+\}\.\n  \{label,\d+\}\.\n    \{line,\[[^\]]*\]\}\.\n    \{func_info,[^\n]*\}\.\n  \{label,\d+\}\.\n)', s)
assert m, "main/0 head not found"
ins=m.group(1)+"    {get_record_field,{x,0},{atom,reach},{atom,field},{integer,1},{x,1}}.\n"
s=s.replace(m.group(1), ins, 1); open('reach_op191.S','w').write(s); print("inserted after:", m.group(1).splitlines()[-1])
PY
echo "== erlc from .S =="; erlc reach_op191.S && ls -la reach_op191.beam
echo "== walker on op191 =="; escript $T/reach_otp.escript reach_op191.beam | grep -E "^(module|walk|op\s+191)"
echo "== beam_disasm on op191 =="; erl -noshell -eval 'R = (catch beam_disasm:file("reach_op191.beam")), case R of {beam_file,_,_,_,_,Fs} -> [io:format("~p~n",[I]) || {function,main,0,_,Is} <- Fs, I <- Is]; _ -> io:format("disasm_failed ~p~n",[R]) end, halt().'
echo "== variant >=192: patch the single 0xBF opcode byte in the Code chunk to 0xC8 (200) =="
python3 - <<'PY'
import struct
b=bytearray(open('reach_op191.beam','rb').read())
assert b[:4]==b'FOR1' and b[8:12]==b'BEAM'
i=12; code=None
while i<len(b):
    tag=b[i:i+4]; n=struct.unpack('>I',b[i+4:i+8])[0]; body=(i+8,i+8+n)
    if tag==b'Code': code=body
    i=i+8+((n+3)//4)*4
s,e=code; sub=struct.unpack('>I',b[s:s+4])[0]; cs=s+4+sub
pos=[k for k in range(cs,e) if b[k]==0xBF]
print("0xBF positions in code:",pos)
assert len(pos)==1, "need exactly one 0xBF to patch unambiguously"
b[pos[0]]=0xC8; open('reach_op200.beam','wb').write(b); print("wrote reach_op200.beam")
PY
echo "== walker on op200 =="; escript $T/reach_otp.escript reach_op200.beam | grep -E "^(module|walk)"
echo "== beam_disasm on op200 =="; erl -noshell -eval 'R = (catch beam_disasm:file("reach_op200.beam")), case R of {beam_file,_,_,_,_,_} -> io:format("disasm_ok~n"); _ -> io:format("disasm_failed ~P~n",[R,8]) end, halt().'
ls -la $D
