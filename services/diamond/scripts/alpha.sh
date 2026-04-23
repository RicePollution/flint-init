#!/usr/bin/env bash
set -e
mkdir -p /tmp/flint-test/diamond
echo $$ > /tmp/flint-test/diamond/alpha.pid
echo "[diamond/alpha] started, pid=$$"
sleep 30
