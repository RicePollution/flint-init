#!/usr/bin/env bash
set -e
mkdir -p /tmp/flint-test/deep_chain
echo $$ > /tmp/flint-test/deep_chain/delta.pid
echo "[deep_chain/delta] started, pid=$$"
sleep 30
