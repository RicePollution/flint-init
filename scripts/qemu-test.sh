#!/usr/bin/env bash
# scripts/qemu-test.sh — build flint-init, package an initramfs, boot in QEMU.
#
# Usage: bash scripts/qemu-test.sh
#
# Requirements: cargo, qemu-system-x86_64, cpio, gzip, ldd
# Run as a normal user; QEMU does not need root.
#
# FLINT_ON_EXIT=halt is passed on the kernel cmdline so the VM exits cleanly
# once the executor returns (e.g., after services die or SIGTERM is received).
# Remove it for production-style behaviour where flint-init reaps zombies forever.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
INITRAMFS_DIR="$(mktemp -d /tmp/flint-initramfs-XXXXXX)"
OUTPUT_CPIO="/tmp/flint-initramfs.cpio.gz"
KERNEL="/boot/vmlinuz-linux"

cleanup() { rm -rf "$INITRAMFS_DIR"; }
trap cleanup EXIT

echo "[qemu-test] building flint-init (release)..."
cargo build --release --manifest-path "$REPO_ROOT/Cargo.toml"
BINARY="$REPO_ROOT/target/release/flint-init"

echo "[qemu-test] scaffolding initramfs in $INITRAMFS_DIR..."
# Essential directory structure
mkdir -p "$INITRAMFS_DIR"/{proc,sys,dev,run,tmp}
mkdir -p "$INITRAMFS_DIR"/{bin,sbin,usr/bin,usr/sbin,lib,lib64,usr/lib,usr/lib64}
mkdir -p "$INITRAMFS_DIR"/lib/x86_64-linux-gnu
mkdir -p "$INITRAMFS_DIR"/services/real

# flint-init binary as /init
install -m 755 "$BINARY" "$INITRAMFS_DIR/init"

# Service definitions
cp "$REPO_ROOT"/services/real/*.toml "$INITRAMFS_DIR/services/real/"

# --- copy a binary and all its ldd-reported shared libs ---
copy_binary() {
    local src="$1"
    local dst_dir="$2"
    local name
    name="$(basename "$src")"
    install -D -m 755 "$src" "$dst_dir/$name"

    # Copy each shared library dependency
    ldd "$src" 2>/dev/null | awk '{
        # ldd lines look like:
        #   libfoo.so.1 => /path/to/libfoo.so.1 (0x...)
        #   /lib64/ld-linux-x86-64.so.2 (0x...)
        if ($2 == "=>") { if ($3 != "not") print $3 }
        else if ($1 ~ /^\//) print $1
    }' | while read -r lib; do
        [ -f "$lib" ] || continue
        # Preserve the original path inside the initramfs
        local dest="$INITRAMFS_DIR$lib"
        mkdir -p "$(dirname "$dest")"
        [ -f "$dest" ] || cp -a "$lib" "$dest"
    done
}

echo "[qemu-test] copying service binaries and their libraries..."
copy_binary /usr/bin/udevd         "$INITRAMFS_DIR/usr/bin"
copy_binary /usr/bin/dbus-daemon   "$INITRAMFS_DIR/usr/bin"
copy_binary /usr/bin/NetworkManager "$INITRAMFS_DIR/usr/bin"

# NetworkManager loads plugins via dlopen — copy the whole plugin dir as fallback
if [ -d /usr/lib/NetworkManager ]; then
    echo "[qemu-test] copying /usr/lib/NetworkManager plugins..."
    cp -a /usr/lib/NetworkManager "$INITRAMFS_DIR/usr/lib/"
    # Also grab any libs those plugins depend on
    find /usr/lib/NetworkManager -name '*.so*' | while read -r plugin; do
        ldd "$plugin" 2>/dev/null | awk '{
            if ($2 == "=>") { if ($3 != "not") print $3 }
            else if ($1 ~ /^\//) print $1
        }' | while read -r lib; do
            [ -f "$lib" ] || continue
            local dest="$INITRAMFS_DIR$lib"
            mkdir -p "$(dirname "$dest")"
            [ -f "$dest" ] || cp -a "$lib" "$dest"
        done
    done
fi

# dbus needs its system config and machine-id
if [ -d /usr/share/dbus-1 ]; then
    cp -a /usr/share/dbus-1 "$INITRAMFS_DIR/usr/share/"
fi
if [ -f /etc/machine-id ]; then
    mkdir -p "$INITRAMFS_DIR/etc"
    cp /etc/machine-id "$INITRAMFS_DIR/etc/"
fi
mkdir -p "$INITRAMFS_DIR/run/dbus"

echo "[qemu-test] packing initramfs -> $OUTPUT_CPIO..."
(cd "$INITRAMFS_DIR" && find . | cpio -o -H newc 2>/dev/null | gzip -9 > "$OUTPUT_CPIO")
echo "[qemu-test] initramfs size: $(du -sh "$OUTPUT_CPIO" | cut -f1)"

echo "[qemu-test] booting in QEMU..."
echo "[qemu-test] kernel: $KERNEL"
echo "[qemu-test] press Ctrl-A X to exit QEMU at any time"
echo "---"
exec qemu-system-x86_64 \
    -kernel "$KERNEL" \
    -initrd "$OUTPUT_CPIO" \
    -append "init=/init FLINT_ON_EXIT=halt console=ttyS0 quiet" \
    -nographic \
    -m 256M \
    -no-reboot
