#!/usr/bin/env bash
set -e
mkdir -p /tmp/flint-test/wide_fanout
echo $$ > /tmp/flint-test/wide_fanout/beta.pid
echo "[wide_fanout/beta] started, pid=$$"
sleep 30
