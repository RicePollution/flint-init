#!/usr/bin/env bash
set -e
mkdir -p /tmp/flint-test/deep_chain
echo $$ > /tmp/flint-test/deep_chain/beta.pid
echo "[deep_chain/beta] started, pid=$$"
sleep 30
