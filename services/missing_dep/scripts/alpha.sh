#!/usr/bin/env bash
set -e
mkdir -p /tmp/flint-test/missing_dep
echo $$ > /tmp/flint-test/missing_dep/alpha.pid
echo "[missing_dep/alpha] started, pid=$$"
sleep 30
