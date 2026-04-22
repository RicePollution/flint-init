#!/usr/bin/env bash
set -e
mkdir -p /tmp/flint-test
echo $$ > /tmp/flint-test/alpha.pid
echo "[alpha] started, pid=$$"
sleep 30
