#!/usr/bin/env bash
# scripts/create-arch-vm.sh — build a minimal Arch Linux (systemd) QEMU disk image.
#
# Uses pacstrap with Arch mirrors (not Artix) to get systemd instead of OpenRC.
# Produces arch.qcow2 ready for measure-systemd-installed.sh.
#
# Requirements (must run as root):
#   qemu-img, qemu-nbd, parted, mkfs.ext4, pacstrap, arch-chroot
#
# Usage: sudo bash scripts/create-arch-vm.sh [output.qcow2]

set -euo pipefail

if [ "$(id -u)" -ne 0 ]; then
    echo "error: must run as root (uses qemu-nbd, mount, pacstrap)" >&2
    exit 1
fi

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DISK_IMAGE="${1:-$REPO_ROOT/arch.qcow2}"
MOUNT_DIR="$(mktemp -d /tmp/arch-root-XXXXXX)"

find_free_nbd() {
    for dev in /dev/nbd{1..15}; do
        if [ "$(cat /sys/block/${dev##/dev/}/size 2>/dev/null)" = "0" ]; then
            echo "$dev"; return
        fi
    done
    echo "/dev/nbd0"
}
modprobe nbd max_part=8
NBD_DEV="$(find_free_nbd)"
echo "[create-arch-vm] using $NBD_DEV"
PACMAN_CONF="/tmp/pacman-arch.conf"

cleanup() {
    umount -R "$MOUNT_DIR" 2>/dev/null || true
    qemu-nbd -d "$NBD_DEV" 2>/dev/null || true
    rmdir "$MOUNT_DIR" 2>/dev/null || true
}
trap cleanup EXIT

# ---- 1. Write an Arch-only pacman.conf so pacstrap pulls systemd, not OpenRC ----
# Artix's repos replace the init system; using Arch repos gives us systemd's base.
cat > "$PACMAN_CONF" << 'EOF'
[options]
Architecture = x86_64
HoldPkg     = pacman glibc
CheckSpace
SigLevel = Required DatabaseOptional

[core]
Server = https://mirrors.kernel.org/archlinux/$repo/os/$arch
Server = https://mirrors.mit.edu/archlinux/$repo/os/$arch
Server = https://mirror.cs.uchicago.edu/archlinux/$repo/os/$arch

[extra]
Server = https://mirrors.kernel.org/archlinux/$repo/os/$arch
Server = https://mirrors.mit.edu/archlinux/$repo/os/$arch
Server = https://mirror.cs.uchicago.edu/archlinux/$repo/os/$arch
EOF

echo "[create-arch-vm] pacman.conf written (Arch repos → systemd base)"

# ---- 2. Create and partition the disk ----
echo "[create-arch-vm] creating $DISK_IMAGE (8G)..."
qemu-img create -f qcow2 "$DISK_IMAGE" 8G

echo "[create-arch-vm] connecting via qemu-nbd..."
qemu-nbd --connect="$NBD_DEV" "$DISK_IMAGE"
sleep 2

echo "[create-arch-vm] partitioning (single ext4 root)..."
parted -s "$NBD_DEV" mklabel msdos mkpart primary ext4 1MiB 100%
sleep 1

echo "[create-arch-vm] formatting ${NBD_DEV}p1 as ext4..."
mkfs.ext4 -F "${NBD_DEV}p1"

echo "[create-arch-vm] mounting..."
mount "${NBD_DEV}p1" "$MOUNT_DIR"

# ---- 3. Install Arch base with systemd ----
echo "[create-arch-vm] running pacstrap with Arch repos (base openssh syslog-ng networkmanager)..."
# Omit 'linux' — we boot with the host kernel via -kernel, saving ~150 MB.
# -C: use our custom pacman.conf (Arch repos with systemd)
# -K: initialise a fresh pacman keyring inside the chroot
pacstrap -C "$PACMAN_CONF" -K "$MOUNT_DIR" base openssh syslog-ng networkmanager

# ---- 4. Configure the installation ----
echo "[create-arch-vm] writing /etc/fstab..."
cat > "$MOUNT_DIR/etc/fstab" << 'EOF'
/dev/vda1   /     ext4   defaults,relatime   0  1
tmpfs       /tmp  tmpfs  defaults            0  0
EOF

echo "[create-arch-vm] setting hostname..."
echo "arch-systemd" > "$MOUNT_DIR/etc/hostname"

echo "[create-arch-vm] removing root password (testing only)..."
arch-chroot "$MOUNT_DIR" passwd -d root

echo "[create-arch-vm] generating ssh host keys..."
arch-chroot "$MOUNT_DIR" ssh-keygen -A

# ---- 5. Configure serial console getty for measurement ----
# systemd ships a getty template; we just need to enable the ttyS0 instance.
echo "[create-arch-vm] enabling serial-getty@ttyS0..."
arch-chroot "$MOUNT_DIR" systemctl enable serial-getty@ttyS0.service

# Also enable sshd and networkmanager so the boot sequence resembles flint-init's
echo "[create-arch-vm] enabling sshd and NetworkManager..."
arch-chroot "$MOUNT_DIR" systemctl enable sshd.service
arch-chroot "$MOUNT_DIR" systemctl enable NetworkManager.service

# ---- 6. Finish ----
sync
echo "[create-arch-vm] done. Image: $DISK_IMAGE"
echo "[create-arch-vm] boot with: sudo bash scripts/measure-systemd-installed.sh $DISK_IMAGE"
