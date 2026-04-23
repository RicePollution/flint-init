#!/usr/bin/env bash
set -e
mkdir -p /tmp/flint-test/diamond
echo $$ > /tmp/flint-test/diamond/beta.pid
echo "[diamond/beta] started, pid=$$"
sleep 30
