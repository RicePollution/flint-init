#!/usr/bin/env bash
# scripts/create-artix-vm.sh — build a minimal Artix QEMU disk image.
#
# Requirements (must run as root):
#   qemu-img, qemu-nbd, parted, mkfs.ext4, pacstrap (arch-install-scripts),
#   arch-chroot, ssh-keygen
#
# Usage: sudo bash scripts/create-artix-vm.sh [output.qcow2]

set -euo pipefail

if [ "$(id -u)" -ne 0 ]; then
    echo "error: must run as root (uses qemu-nbd, mount, pacstrap)" >&2
    exit 1
fi

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DISK_IMAGE="${1:-$REPO_ROOT/artix.qcow2}"
MOUNT_DIR="$(mktemp -d /tmp/artix-root-XXXXXX)"
NBD_DEV="/dev/nbd0"

cleanup() {
    umount -R "$MOUNT_DIR" 2>/dev/null || true
    qemu-nbd -d "$NBD_DEV" 2>/dev/null || true
    rmdir "$MOUNT_DIR" 2>/dev/null || true
}
trap cleanup EXIT

echo "[create-vm] creating $DISK_IMAGE (8G)..."
qemu-img create -f qcow2 "$DISK_IMAGE" 8G

echo "[create-vm] connecting via qemu-nbd..."
modprobe nbd max_part=8
qemu-nbd --connect="$NBD_DEV" "$DISK_IMAGE"
sleep 1

echo "[create-vm] partitioning (single ext4 root)..."
parted -s "$NBD_DEV" mklabel msdos mkpart primary ext4 1MiB 100%
sleep 1

echo "[create-vm] formatting /dev/nbd0p1 as ext4..."
mkfs.ext4 -F "${NBD_DEV}p1"

echo "[create-vm] mounting..."
mount "${NBD_DEV}p1" "$MOUNT_DIR"

echo "[create-vm] running pacstrap (base linux openssh syslog-ng networkmanager)..."
pacstrap -K "$MOUNT_DIR" base linux openssh syslog-ng networkmanager

echo "[create-vm] writing /etc/fstab..."
cat > "$MOUNT_DIR/etc/fstab" << 'EOF'
# flint-init stage 4 fstab
/dev/vda1   /     ext4   defaults,relatime   0  1
tmpfs       /tmp  tmpfs  defaults            0  0
EOF

echo "[create-vm] setting hostname..."
echo "flint-artix" > "$MOUNT_DIR/etc/hostname"

echo "[create-vm] removing root password (testing only)..."
arch-chroot "$MOUNT_DIR" passwd -d root

echo "[create-vm] generating ssh host keys..."
arch-chroot "$MOUNT_DIR" ssh-keygen -A

echo "[create-vm] installing flint-init..."
bash "$REPO_ROOT/scripts/install-flint-into-vm.sh" "$MOUNT_DIR"

echo "[create-vm] done. Image: $DISK_IMAGE"
echo "[create-vm] boot with: bash scripts/boot-artix-vm.sh $DISK_IMAGE"
