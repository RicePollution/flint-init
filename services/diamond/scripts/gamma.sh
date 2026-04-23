#!/usr/bin/env bash
set -e
mkdir -p /tmp/flint-test/diamond
echo $$ > /tmp/flint-test/diamond/gamma.pid
echo "[diamond/gamma] started, pid=$$"
sleep 30
