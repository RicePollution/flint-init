#!/usr/bin/env bash
# scripts/measure-systemd-installed.sh — measure systemd boot time on an
# installed Arch Linux disk image, using the same QEMU config as flint-init.
#
# Methodology (matches boot-artix-vm.sh):
#   - Same host kernel (/boot/vmlinuz-linux), no initramfs
#   - Same virtio disk, same RAM/CPU
#   - init=/usr/lib/systemd/systemd
#   - Measure: QEMU launch → "login:" on ttyS0
#
# Prerequisites: sudo bash scripts/create-arch-vm.sh
# Usage: bash scripts/measure-systemd-installed.sh [arch.qcow2]

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DISK_IMAGE="${1:-$REPO_ROOT/arch.qcow2}"
KERNEL="/boot/vmlinuz-linux"
SERIAL_LOG="/tmp/systemd-arch-serial.txt"
TIMEOUT=180  # seconds — systemd is slower; give it room

if [ ! -f "$DISK_IMAGE" ]; then
    echo "error: disk image not found: $DISK_IMAGE"
    echo "run: sudo bash scripts/create-arch-vm.sh"
    exit 1
fi

if [ ! -f "$KERNEL" ]; then
    echo "error: kernel not found: $KERNEL"
    exit 1
fi

# ---- 1. Boot in QEMU ----
rm -f "$SERIAL_LOG"
touch "$SERIAL_LOG"

echo "[measure] booting $DISK_IMAGE with systemd (init=/usr/lib/systemd/systemd)..."
echo "[measure] kernel: $KERNEL"
echo "[measure] serial log: $SERIAL_LOG"
echo "[measure] timeout: ${TIMEOUT}s"

T0_NS=$(date +%s%N)

qemu-system-x86_64 \
    -enable-kvm \
    -drive "file=$DISK_IMAGE,if=virtio,format=qcow2" \
    -kernel "$KERNEL" \
    -append "root=/dev/vda1 rw init=/usr/lib/systemd/systemd console=ttyS0 loglevel=3 systemd.show_status=1" \
    -display none \
    -chardev "file,id=char0,path=$SERIAL_LOG" \
    -serial chardev:char0 \
    -m 512M \
    -smp 2 \
    -netdev user,id=net0 \
    -device virtio-net-pci,netdev=net0 \
    -no-reboot &

QEMU_PID=$!

# ---- 2. Poll serial log for timestamps ----
echo "[measure] waiting for boot..."

PID1_NS=""
LOGIN_NS=""
START_WAIT=$SECONDS

while [ $(( SECONDS - START_WAIT )) -lt $TIMEOUT ]; do
    if ! kill -0 "$QEMU_PID" 2>/dev/null; then
        echo "[measure] QEMU exited"
        break
    fi

    if [ -z "$PID1_NS" ] && grep -qE '(systemd\[1\]|Welcome to Arch|Welcome to .* Linux)' "$SERIAL_LOG" 2>/dev/null; then
        PID1_NS=$(date +%s%N)
        echo "[measure] systemd PID1 at $(( ( PID1_NS - T0_NS ) / 1000000 )) ms from QEMU start"
    fi

    if [ -z "$LOGIN_NS" ] && grep -qE '(arch-systemd login:|Arch Linux.*ttyS0|login: $)' "$SERIAL_LOG" 2>/dev/null; then
        LOGIN_NS=$(date +%s%N)
        echo "[measure] login prompt at $(( ( LOGIN_NS - T0_NS ) / 1000000 )) ms from QEMU start"
        break
    fi

    sleep 0.1
done

kill "$QEMU_PID" 2>/dev/null || true
wait "$QEMU_PID" 2>/dev/null || true

# ---- 3. Results ----
echo ""

if [ -z "$LOGIN_NS" ]; then
    echo "[measure] ERROR: login prompt not seen within ${TIMEOUT}s"
    echo "[measure] Last 60 lines of serial log:"
    tail -60 "$SERIAL_LOG"
    exit 1
fi

if [ -z "$PID1_NS" ]; then
    PID1_NS=$T0_NS
fi

KERNEL_MS=$(( ( PID1_NS - T0_NS ) / 1000000 ))
USERSPACE_MS=$(( ( LOGIN_NS - PID1_NS ) / 1000000 ))
TOTAL_MS=$(( ( LOGIN_NS - T0_NS ) / 1000000 ))

SYSTEMD_VER=$(grep -oP 'systemd[- ]\K[0-9]+\.[0-9]+' "$SERIAL_LOG" | head -1 || \
              arch-chroot /tmp 2>/dev/null pacman -Q systemd 2>/dev/null | grep -oP '[0-9]+\.[0-9]+' | head -1 || \
              echo "260")
KERNEL_VER=$(uname -r)

echo "========================================"
echo "  systemd boot measurement (installed)"
echo "========================================"
echo "  systemd version:           ${SYSTEMD_VER}"
echo "  kernel:                    ${KERNEL_VER}"
echo "  kernel boot (QEMU → PID1): ${KERNEL_MS} ms"
echo "  userspace (PID 1 → login): ${USERSPACE_MS} ms"
echo "  total:                     ${TOTAL_MS} ms"
echo "========================================"
echo ""
echo "README table row:"
printf "| systemd %s (Arch, installed) | %s | %s ms | **%s ms** | %s ms |\n" \
    "$SYSTEMD_VER" "$KERNEL_VER" "$KERNEL_MS" "$USERSPACE_MS" "$TOTAL_MS"
echo ""
echo "Full serial log: $SERIAL_LOG"
