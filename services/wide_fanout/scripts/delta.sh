#!/usr/bin/env bash
set -e
mkdir -p /tmp/flint-test/wide_fanout
echo $$ > /tmp/flint-test/wide_fanout/delta.pid
echo "[wide_fanout/delta] started, pid=$$"
sleep 30
