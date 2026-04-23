#!/usr/bin/env bash
set -e
mkdir -p /tmp/flint-test/wide_fanout
echo $$ > /tmp/flint-test/wide_fanout/alpha.pid
echo "[wide_fanout/alpha] started, pid=$$"
sleep 30
