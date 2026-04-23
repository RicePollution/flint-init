#!/usr/bin/env bash
set -e
mkdir -p /tmp/flint-test/deep_chain
echo $$ > /tmp/flint-test/deep_chain/alpha.pid
echo "[deep_chain/alpha] started, pid=$$"
sleep 30
