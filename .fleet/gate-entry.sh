#!/bin/bash
set -u
export PATH=$HOME/.cargo/bin:$PATH
ulimit -n 65536 2>/dev/null
echo "GATE SCOPE: beamr's own CI verdict harness (gates.json), the real mechanism"
./scripts/ci-verdict.sh gate-rc gates.json || exit 1
echo GATE-GREEN
