#!/usr/bin/env bash
set -e
mkdir -p /tmp/flint-test
echo $$ > /tmp/flint-test/beta.pid
echo "[beta] started, pid=$$"
sleep 30
