#!/usr/bin/env bash
set -e
mkdir -p /tmp/flint-test/cycle
echo $$ > /tmp/flint-test/cycle/alpha.pid
echo "[cycle/alpha] started, pid=$$"
sleep 30
