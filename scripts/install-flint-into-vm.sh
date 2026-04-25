#!/usr/bin/env bash
# scripts/install-flint-into-vm.sh — install flint-init into a mounted VM root.
#
# Usage (mounted dir):   bash scripts/install-flint-into-vm.sh /mnt/artix
# Usage (auto via nbd):  sudo bash scripts/install-flint-into-vm.sh  [artix.qcow2]
#
# In the second form the script connects artix.qcow2 via qemu-nbd, installs,
# then disconnects.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

echo "[install] building release binaries..."
cargo build --release --manifest-path "$REPO_ROOT/Cargo.toml"

if [ $# -ge 1 ] && [ -d "$1" ]; then
    # First argument is an already-mounted directory.
    ROOT="$1"
    MOUNTED_HERE=false
else
    # Auto-mount artix.qcow2 via qemu-nbd.
    if [ "$(id -u)" -ne 0 ]; then
        echo "error: auto-mount mode requires root (uses qemu-nbd)" >&2
        exit 1
    fi
    DISK_IMAGE="${1:-$REPO_ROOT/artix.qcow2}"
    if [ ! -f "$DISK_IMAGE" ]; then
        echo "error: disk image not found: $DISK_IMAGE" >&2
        exit 1
    fi
    ROOT="$(mktemp -d /tmp/artix-install-XXXXXX)"
    MOUNTED_HERE=true
    modprobe nbd max_part=8
    qemu-nbd --connect=/dev/nbd0 "$DISK_IMAGE"
    sleep 1
    mount /dev/nbd0p1 "$ROOT"
fi

cleanup() {
    if [ "$MOUNTED_HERE" = true ]; then
        umount "$ROOT" 2>/dev/null || true
        qemu-nbd -d /dev/nbd0 2>/dev/null || true
        rmdir "$ROOT" 2>/dev/null || true
    fi
}
trap cleanup EXIT

echo "[install] copying binaries..."
install -D -m 755 "$REPO_ROOT/target/release/flint-init" "$ROOT/usr/sbin/flint-init"
install -D -m 755 "$REPO_ROOT/target/release/flint-ctl"  "$ROOT/usr/bin/flint-ctl"

echo "[install] copying service definitions..."
mkdir -p "$ROOT/etc/flint/services"
cp "$REPO_ROOT/services/artix/"*.toml "$ROOT/etc/flint/services/"

echo "[install] installed:"
ls -la "$ROOT/usr/sbin/flint-init" "$ROOT/usr/bin/flint-ctl"
ls "$ROOT/etc/flint/services/"

# Flush all writes to disk before the caller unmounts.
sync
