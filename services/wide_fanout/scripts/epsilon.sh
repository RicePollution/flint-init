#!/usr/bin/env bash
set -e
mkdir -p /tmp/flint-test/wide_fanout
echo $$ > /tmp/flint-test/wide_fanout/epsilon.pid
echo "[wide_fanout/epsilon] started, pid=$$"
sleep 30
