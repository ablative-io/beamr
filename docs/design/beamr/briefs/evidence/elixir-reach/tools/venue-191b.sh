#!/bin/bash
set -euo pipefail
ROOT=/home/aion/reach/elixir-reach-20260903; T=$ROOT/tools; D=$ROOT/controls191; mkdir -p $D; cd $D
rm -f reach_op191.beam reach_op200.beam
cp $ROOT/controls/reach_lc.erl reach_opbase.erl; sed -i 's/-module(reach_lc)\./-module(reach_opbase)./' reach_opbase.erl; erlc reach_opbase.erl
echo "== offsets in reach_opbase.beam (first allocate=12 is main/0's first real instruction) =="
REACH_OFFSETS=1 escript $T/reach_otp.escript reach_opbase.beam | grep -E "^off" | head -12
OFF=$(REACH_OFFSETS=1 escript $T/reach_otp.escript reach_opbase.beam | awk -F'\t' '$1=="off" && $3==12 {print $2; exit}')
echo "patch offset (code-relative)=$OFF"
python3 - "$OFF" <<'PY'
import struct,sys
off=int(sys.argv[1]); b=bytearray(open('reach_opbase.beam','rb').read())
assert b[:4]==b'FOR1' and b[8:12]==b'BEAM'
i=12; code=None
while i<len(b):
    tag=b[i:i+4]; n=struct.unpack('>I',b[i+4:i+8])[0]
    if tag==b'Code': code=(i+8,i+8+n)
    i=i+8+((n+3)//4)*4
s,e=code; sub=struct.unpack('>I',b[s:s+4])[0]; cs=s+4+sub
assert b[cs+off]==12, f"byte at offset is {b[cs+off]}, expected 12 (allocate)"
for val,name in ((191,'reach_op191'),(200,'reach_op200')):
    c=bytearray(b); c[cs+off]=val; open(name+'.beam','wb').write(c); print("wrote",name+'.beam',"opcode byte",val,"at file offset",cs+off)
PY
for m in reach_op191 reach_op200; do
  echo "== walker on $m =="; escript $T/reach_otp.escript $m.beam | grep -E "^(module|walk|op\s+(191|200)\s)"
  echo "== beam_disasm on $m =="; erl -noshell -eval "R = (catch beam_disasm:file(\"$m.beam\")), case R of {beam_file,_,_,_,_,Fs} -> io:format(\"disasm_ok~n\"), [io:format(\"  ~P~n\",[I,6]) || {function,main,0,_,Is} <- Fs, I <- Is]; _ -> io:format(\"disasm_failed ~P~n\",[R,10]) end, halt()."
done
ls -la $D
