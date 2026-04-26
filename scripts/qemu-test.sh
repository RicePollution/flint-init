#!/usr/bin/env bash
# scripts/qemu-test.sh — build flint-init, package an initramfs, boot in QEMU.
# Uses services/initramfs/ (minimal 4-service set: udev, dbus, nm-priv-helper, NM).
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
mkdir -p "$INITRAMFS_DIR"/var/lib/NetworkManager
ln -sf /run "$INITRAMFS_DIR/var/run"
mkdir -p "$INITRAMFS_DIR"/{bin,sbin,usr/bin,usr/sbin,usr/lib}
# Mirror host symlink layout: /lib → usr/lib, /lib64 → usr/lib, /usr/lib64 → lib
ln -sf usr/lib "$INITRAMFS_DIR/lib"
ln -sf usr/lib "$INITRAMFS_DIR/lib64"
ln -sf lib     "$INITRAMFS_DIR/usr/lib64"
mkdir -p "$INITRAMFS_DIR"/services/initramfs
mkdir -p "$INITRAMFS_DIR/run/NetworkManager"

# flint-init binary as /init
install -m 755 "$BINARY" "$INITRAMFS_DIR/init"

# Copy flint-init's own shared library dependencies (dynamic linker, libc, libgcc)
ldd "$BINARY" 2>/dev/null | awk '{
    if ($2 == "=>") { if ($3 != "not") print $3 }
    else if ($1 ~ /^\//) print $1
}' | while read -r lib; do
    [ -f "$lib" ] || continue
    dest="$INITRAMFS_DIR$lib"
    mkdir -p "$(dirname "$dest")"
    [ -f "$dest" ] || cp -L "$lib" "$dest"
done

# Service definitions
cp "$REPO_ROOT"/services/initramfs/*.toml "$INITRAMFS_DIR/services/initramfs/"

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
        [ -f "$dest" ] || cp -L "$lib" "$dest"
    done
}

echo "[qemu-test] copying service binaries and their libraries..."
copy_binary /usr/bin/udevd           "$INITRAMFS_DIR/usr/bin"
copy_binary /usr/bin/dbus-daemon     "$INITRAMFS_DIR/usr/bin"
copy_binary /usr/bin/NetworkManager  "$INITRAMFS_DIR/usr/bin"
copy_binary /bin/bash                "$INITRAMFS_DIR/bin"
copy_binary /usr/bin/dbus-send       "$INITRAMFS_DIR/usr/bin"
# nm-priv-helper: privilege-isolation helper required by NM 1.40+.
# Lives in /usr/lib, not /usr/bin.
copy_binary /usr/lib/nm-priv-helper  "$INITRAMFS_DIR/usr/lib"

# nm-priv-helper wrapper: starts nm-priv-helper, waits for it to register on
# D-Bus, then writes a pidfile so flint-init knows it is ready.
# This lets NM find nm-priv-helper already registered instead of triggering
# D-Bus activation, which requires the setuid launch-helper we cannot copy.
cat > "$INITRAMFS_DIR/usr/lib/nm-priv-helper-wrapper" << 'WRAPPER'
#!/bin/bash
/usr/lib/nm-priv-helper &
NM_PRIV_PID=$!
# Poll until nm-priv-helper's D-Bus name is registered.
until [[ $(dbus-send --system --dest=org.freedesktop.DBus --type=method_call \
    --print-reply /org/freedesktop/DBus org.freedesktop.DBus.ListNames 2>/dev/null) \
    == *nm_priv_helper* ]]; do
    :
done
echo $NM_PRIV_PID > /run/nm-priv-helper.pid
wait $NM_PRIV_PID
WRAPPER
chmod 755 "$INITRAMFS_DIR/usr/lib/nm-priv-helper-wrapper"

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
            dest="$INITRAMFS_DIR$lib"
            mkdir -p "$(dirname "$dest")"
            [ -f "$dest" ] || cp -L "$lib" "$dest"
        done
    done
fi

# NetworkManager config:
#   no-auto-default=*   — don't create automatic default wired connections.
#   auth-polkit=false   — polkit is absent in initramfs; allow all requests.
mkdir -p "$INITRAMFS_DIR/etc/NetworkManager"
cat > "$INITRAMFS_DIR/etc/NetworkManager/NetworkManager.conf" << 'EOF'
[main]
plugins=
no-auto-default=*
auth-polkit=false
EOF

# dbus needs its system config and machine-id
if [ -d /usr/share/dbus-1 ]; then
    mkdir -p "$INITRAMFS_DIR/usr/share"
    cp -a /usr/share/dbus-1 "$INITRAMFS_DIR/usr/share/"
    # Remove all D-Bus activation entries — they carry SystemdService= which
    # makes dbus-daemon try to contact a non-existent systemd, hanging callers.
    # nm-priv-helper is pre-started by flint-init instead of D-Bus activation.
    rm -rf "$INITRAMFS_DIR/usr/share/dbus-1/system-services/"
    # Remove <user>dbus</user> so dbus-daemon stays root.
    # The real setuid launch-helper can't be copied; running as root avoids it.
    sed -i 's|<user>dbus</user>||g' "$INITRAMFS_DIR/usr/share/dbus-1/system.conf"
fi
mkdir -p "$INITRAMFS_DIR/etc"
if [ -d /etc/dbus-1 ]; then
    cp -a /etc/dbus-1 "$INITRAMFS_DIR/etc/"
fi
if [ -f /etc/machine-id ]; then
    cp /etc/machine-id "$INITRAMFS_DIR/etc/"
fi
mkdir -p "$INITRAMFS_DIR/run/dbus"

# hostname — NM reads this during startup
echo "flint-vm" > "$INITRAMFS_DIR/etc/hostname"

# dbus-daemon and other services need passwd/group for UID/GID lookups
cp /etc/passwd  "$INITRAMFS_DIR/etc/passwd"
cp /etc/group   "$INITRAMFS_DIR/etc/group"
cp /etc/nsswitch.conf "$INITRAMFS_DIR/etc/nsswitch.conf"
# libc loads libnss_files.so.2 at runtime via dlopen — not visible to ldd
cp -L /usr/lib/libnss_files.so.2 "$INITRAMFS_DIR/usr/lib/libnss_files.so.2"

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
    -append "init=/init FLINT_ON_EXIT=halt GIO_MODULE_DIR=/dev/null console=ttyS0 -- /services/initramfs" \
    -nographic \
    -m 256M \
    -no-reboot \
    -object rng-random,id=rng0,filename=/dev/urandom \
    -device virtio-rng-pci,rng=rng0 \
    -chardev socket,id=mon,path=/tmp/qemu-monitor.sock,server=on,wait=off \
    -monitor chardev:mon
