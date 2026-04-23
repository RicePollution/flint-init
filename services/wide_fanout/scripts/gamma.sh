#!/usr/bin/env bash
set -e
mkdir -p /tmp/flint-test/wide_fanout
echo $$ > /tmp/flint-test/wide_fanout/gamma.pid
echo "[wide_fanout/gamma] started, pid=$$"
sleep 30
