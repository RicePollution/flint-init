#!/usr/bin/env bash
set -e
mkdir -p /tmp/flint-test/diamond
echo $$ > /tmp/flint-test/diamond/delta.pid
echo "[diamond/delta] started, pid=$$"
sleep 30
