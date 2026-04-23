#!/usr/bin/env bash
set -e
mkdir -p /tmp/flint-test/wide_fanout
echo $$ > /tmp/flint-test/wide_fanout/zeta.pid
echo "[wide_fanout/zeta] started, pid=$$"
sleep 30
