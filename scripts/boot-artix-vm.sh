#!/usr/bin/env bash
# scripts/boot-artix-vm.sh — boot artix.qcow2 with flint-init as PID 1.
#
# Uses the host kernel (-kernel) — bypasses GRUB entirely.
# Virtio-blk and ext4 are built-in on Arch/Artix kernels so no initramfs needed.
#
# Serial output goes to a log file for inspection.
# Usage: bash scripts/boot-artix-vm.sh [artix.qcow2]

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DISK_IMAGE="${1:-$REPO_ROOT/artix.qcow2}"
KERNEL="/boot/vmlinuz-linux"
SERIAL_LOG="/tmp/flint-artix-serial.txt"

if [ ! -f "$DISK_IMAGE" ]; then
    echo "error: disk image not found: $DISK_IMAGE"
    echo "run: sudo bash scripts/create-artix-vm.sh"
    exit 1
fi

echo "[boot] disk:   $DISK_IMAGE"
echo "[boot] kernel: $KERNEL"
echo "[boot] serial log: $SERIAL_LOG"
echo "[boot] press Ctrl-C to stop QEMU"
echo "---"

qemu-system-x86_64 \
    -drive "file=$DISK_IMAGE,if=virtio,format=qcow2" \
    -kernel "$KERNEL" \
    -append "root=/dev/vda1 rw init=/usr/sbin/flint-init console=ttyS0 loglevel=3" \
    -display none \
    -chardev "file,id=char0,path=$SERIAL_LOG" \
    -serial chardev:char0 \
    -m 512M \
    -smp 2 \
    -netdev user,id=net0 \
    -device virtio-net-pci,netdev=net0 \
    -no-reboot &

QEMU_PID=$!

# Tail the serial log live while QEMU runs.
sleep 1
tail -f "$SERIAL_LOG" &
TAIL_PID=$!

wait $QEMU_PID
kill $TAIL_PID 2>/dev/null || true

echo "---"
echo "[boot] QEMU exited. Full serial log: $SERIAL_LOG"
