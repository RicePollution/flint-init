#!/usr/bin/env bash
# scripts/install-flint-into-vm.sh — DEPRECATED
# This script is now a thin wrapper around install.sh --root <path>.
# Use install.sh directly for new workflows.
#
# Usage (mounted dir):   bash scripts/install-flint-into-vm.sh /mnt/artix
# Usage (auto via nbd):  sudo bash scripts/install-flint-into-vm.sh [artix.qcow2]

set -euo pipefail
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

if [ $# -ge 1 ] && [ -d "$1" ]; then
    # Already-mounted directory — pass straight through
    exec sudo bash "$REPO_ROOT/install.sh" --root "$1"
else
    # Auto-mount artix.qcow2 via qemu-nbd, then install
    if [ "$(id -u)" -ne 0 ]; then
        echo "error: auto-mount mode requires root (uses qemu-nbd)" >&2
        exit 1
    fi
    DISK_IMAGE="${1:-$REPO_ROOT/artix.qcow2}"
    if [ ! -f "$DISK_IMAGE" ]; then
        echo "error: disk image not found: $DISK_IMAGE" >&2
        exit 1
    fi
    ROOT_DIR="$(mktemp -d /tmp/artix-install-XXXXXX)"
    modprobe nbd max_part=8
    qemu-nbd --connect=/dev/nbd0 "$DISK_IMAGE"
    sleep 1
    mount /dev/nbd0p1 "$ROOT_DIR"
    cleanup() {
        umount "$ROOT_DIR" 2>/dev/null || true
        qemu-nbd -d /dev/nbd0 2>/dev/null || true
        rmdir "$ROOT_DIR" 2>/dev/null || true
    }
    trap cleanup EXIT
    bash "$REPO_ROOT/install.sh" --root "$ROOT_DIR"
fi
