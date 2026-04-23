#!/usr/bin/env bash
set -e
mkdir -p /tmp/flint-test/cycle
echo $$ > /tmp/flint-test/cycle/beta.pid
echo "[cycle/beta] started, pid=$$"
sleep 30
