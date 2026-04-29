#!/usr/bin/env bash
# scripts/measure-openrc-installed.sh — measure OpenRC boot time on the same
# installed Artix disk image used for flint-init, so hardware and software
# environment are identical.
#
# Methodology (matches boot-artix-vm.sh):
#   - Same host kernel (/boot/vmlinuz-linux), no initramfs
#   - Same virtio disk, same RAM/CPU
#   - init=/usr/bin/openrc-init instead of flint-init
#   - Measure: QEMU launch → "login:" on ttyS0
#
# Requirements: qemu-system-x86_64, qemu-nbd, sudo (for nbd mount)
# Usage: sudo bash scripts/measure-openrc-installed.sh [artix.qcow2]

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DISK_IMAGE="${1:-$REPO_ROOT/artix.qcow2}"
KERNEL="/boot/vmlinuz-linux"
SERIAL_LOG="/tmp/openrc-artix-serial.txt"
TIMEOUT=180  # seconds

if [ ! -f "$DISK_IMAGE" ]; then
    echo "error: disk image not found: $DISK_IMAGE"
    echo "run: sudo bash scripts/create-artix-vm.sh"
    exit 1
fi

if [ ! -f "$KERNEL" ]; then
    echo "error: kernel not found: $KERNEL"
    exit 1
fi

# ---- 1. Ensure ttyS0 serial getty is configured in /etc/inittab ----
# OpenRC uses inittab for getty; the ttyS0 line is commented out by default.
# We need to enable it so the login prompt appears on the serial console.
# This is a one-time fixup — it doesn't affect flint-init which ignores inittab.

echo "[measure] checking /etc/inittab for ttyS0 getty..."

if [ "$(id -u)" -ne 0 ]; then
    echo "error: must run as root (uses qemu-nbd to configure inittab)" >&2
    exit 1
fi

MOUNT_DIR="$(mktemp -d /tmp/openrc-mount-XXXXXX)"

# Find a free nbd device to avoid stale-state issues on nbd0
find_free_nbd() {
    for dev in /dev/nbd{1..15}; do
        if [ "$(cat /sys/block/${dev##/dev/}/size 2>/dev/null)" = "0" ]; then
            echo "$dev"
            return
        fi
    done
    echo "/dev/nbd0"
}

modprobe nbd max_part=8
NBD_DEV="$(find_free_nbd)"
echo "[measure] using $NBD_DEV"

cleanup_mount() {
    umount "$MOUNT_DIR" 2>/dev/null || true
    qemu-nbd -d "$NBD_DEV" 2>/dev/null || true
    rmdir "$MOUNT_DIR" 2>/dev/null || true
}

qemu-nbd --connect="$NBD_DEV" "$DISK_IMAGE"
sleep 2
mount "${NBD_DEV}p1" "$MOUNT_DIR"

INITTAB="$MOUNT_DIR/etc/inittab"

if [ ! -f "$INITTAB" ]; then
    echo "[measure] /etc/inittab not found — creating it with ttyS0 getty..."
    cat > "$INITTAB" << 'EOF'
# /etc/inittab — OpenRC serial console
::sysinit:/sbin/openrc sysinit
::sysinit:/sbin/openrc boot
::wait:/sbin/openrc default

# Terminals
tty1::respawn:/sbin/agetty 38400 tty1 linux
ttyS0::respawn:/sbin/agetty -L ttyS0 115200 vt100

::ctrlaltdel:/sbin/reboot
::shutdown:/sbin/openrc shutdown
EOF
else
    # Uncomment or add ttyS0 line
    if grep -q 'ttyS0' "$INITTAB"; then
        # Uncomment if it's there but commented out
        sed -i 's|^#\(.*ttyS0.*agetty.*\)|\1|' "$INITTAB"
        echo "[measure] uncommented ttyS0 getty in /etc/inittab"
    else
        # Add it
        echo 'ttyS0::respawn:/sbin/agetty -L ttyS0 115200 vt100' >> "$INITTAB"
        echo "[measure] added ttyS0 getty to /etc/inittab"
    fi
fi

# Ensure agetty.ttyS0 OpenRC service exists (symlink to agetty script).
# Never add the generic 'agetty' service — it requires a port-specific symlink name.
if [ ! -e "$MOUNT_DIR/etc/init.d/agetty.ttyS0" ] && [ -f "$MOUNT_DIR/etc/init.d/agetty" ]; then
    ln -sf /etc/init.d/agetty "$MOUNT_DIR/etc/init.d/agetty.ttyS0"
    echo "[measure] created /etc/init.d/agetty.ttyS0 symlink"
fi
if [ ! -e "$MOUNT_DIR/etc/runlevels/default/agetty.ttyS0" ] && [ -e "$MOUNT_DIR/etc/init.d/agetty.ttyS0" ]; then
    mkdir -p "$MOUNT_DIR/etc/runlevels/default"
    ln -sf /etc/init.d/agetty.ttyS0 "$MOUNT_DIR/etc/runlevels/default/agetty.ttyS0"
    echo "[measure] enabled agetty.ttyS0 in default runlevel"
fi
# Remove generic agetty if it was added previously — it always fails
rm -f "$MOUNT_DIR/etc/runlevels/default/agetty"

sync
umount "$MOUNT_DIR"
qemu-nbd -d "$NBD_DEV"
rmdir "$MOUNT_DIR"
echo "[measure] inittab configured."

# ---- 2. Boot in QEMU ----
rm -f "$SERIAL_LOG"
touch "$SERIAL_LOG"

echo "[measure] booting $DISK_IMAGE with OpenRC (init=/usr/bin/openrc-init)..."
echo "[measure] kernel: $KERNEL"
echo "[measure] serial log: $SERIAL_LOG"

T0_NS=$(date +%s%N)

qemu-system-x86_64 \
    -enable-kvm \
    -drive "file=$DISK_IMAGE,if=virtio,format=qcow2" \
    -kernel "$KERNEL" \
    -append "root=/dev/vda1 rw init=/usr/bin/openrc-init console=ttyS0 loglevel=3" \
    -display none \
    -chardev "file,id=char0,path=$SERIAL_LOG" \
    -serial chardev:char0 \
    -m 512M \
    -smp 2 \
    -netdev user,id=net0 \
    -device virtio-net-pci,netdev=net0 \
    -no-reboot &

QEMU_PID=$!

# ---- 3. Poll serial log for PID 1 exec and login prompt ----
echo "[measure] waiting for boot (timeout: ${TIMEOUT}s)..."

PID1_NS=""
LOGIN_NS=""
START_WAIT=$SECONDS

while [ $(( SECONDS - START_WAIT )) -lt $TIMEOUT ]; do
    if ! kill -0 "$QEMU_PID" 2>/dev/null; then
        echo "[measure] QEMU exited"
        break
    fi

    # OpenRC PID 1 logs "openrc-init" or "OpenRC" early
    if [ -z "$PID1_NS" ] && grep -qiE '(openrc|Starting udev|Caching service)' "$SERIAL_LOG" 2>/dev/null; then
        PID1_NS=$(date +%s%N)
        echo "[measure] OpenRC activity at $(( ( PID1_NS - T0_NS ) / 1000000 )) ms from QEMU start"
    fi

    if [ -z "$LOGIN_NS" ] && grep -qE '(login:|artixlinux login|flint-artix login)' "$SERIAL_LOG" 2>/dev/null; then
        LOGIN_NS=$(date +%s%N)
        echo "[measure] login prompt at $(( ( LOGIN_NS - T0_NS ) / 1000000 )) ms from QEMU start"
        break
    fi

    sleep 0.1
done

kill "$QEMU_PID" 2>/dev/null || true
wait "$QEMU_PID" 2>/dev/null || true

# ---- 4. Results ----
echo ""

if [ -z "$LOGIN_NS" ]; then
    echo "[measure] ERROR: login prompt not seen within ${TIMEOUT}s"
    echo "[measure] Last 60 lines of serial log:"
    tail -60 "$SERIAL_LOG"
    exit 1
fi

if [ -z "$PID1_NS" ]; then
    # Fall back: treat QEMU start as PID1 time (conservative)
    PID1_NS=$T0_NS
fi

KERNEL_MS=$(( ( PID1_NS - T0_NS ) / 1000000 ))
USERSPACE_MS=$(( ( LOGIN_NS - PID1_NS ) / 1000000 ))
TOTAL_MS=$(( ( LOGIN_NS - T0_NS ) / 1000000 ))

OPENRC_VER=$(grep -oP 'OpenRC \K[0-9.]+' "$SERIAL_LOG" | head -1 || echo "???")
KERNEL_VER=$(uname -r)

echo "========================================"
echo "  OpenRC boot measurement (installed)"
echo "========================================"
echo "  OpenRC version:            ${OPENRC_VER}"
echo "  kernel:                    ${KERNEL_VER}"
echo "  kernel boot (QEMU → PID1): ${KERNEL_MS} ms"
echo "  userspace (PID 1 → login): ${USERSPACE_MS} ms"
echo "  total:                     ${TOTAL_MS} ms"
echo "========================================"
echo ""
echo "README table row:"
printf "| OpenRC %s (Artix, installed) | %s | %s ms | **%s ms** | %s ms |\n" \
    "$OPENRC_VER" "$KERNEL_VER" "$KERNEL_MS" "$USERSPACE_MS" "$TOTAL_MS"
echo ""
echo "Full serial log: $SERIAL_LOG"
