#!/usr/bin/env bash
set -e
mkdir -p /tmp/flint-test
echo $$ > /tmp/flint-test/gamma.pid
echo "[gamma] started, pid=$$"
sleep 30
