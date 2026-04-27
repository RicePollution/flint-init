#!/usr/bin/env bash
# scripts/create-openrc-vm.sh — build a minimal Artix Linux + OpenRC QEMU disk image
# for boot-time comparison against flint-init.
#
# Does NOT install flint-init — this is a pure OpenRC baseline.
# Uses the host's pacman.conf (Artix OpenRC repos).
#
# Requirements (must run as root):
#   qemu-img, qemu-nbd, parted, mkfs.ext4, pacstrap, arch-chroot
#
# Usage: sudo bash scripts/create-openrc-vm.sh [output.qcow2]

set -euo pipefail

if [ "$(id -u)" -ne 0 ]; then
    echo "error: must run as root (uses qemu-nbd, mount, pacstrap)" >&2
    exit 1
fi

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DISK_IMAGE="${1:-$REPO_ROOT/artix-openrc.qcow2}"
MOUNT_DIR="$(mktemp -d /tmp/openrc-root-XXXXXX)"

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
echo "[create-openrc-vm] using $NBD_DEV"

cleanup() {
    umount -R "$MOUNT_DIR" 2>/dev/null || true
    qemu-nbd -d "$NBD_DEV" 2>/dev/null || true
    rmdir "$MOUNT_DIR" 2>/dev/null || true
}
trap cleanup EXIT

echo "[create-openrc-vm] creating $DISK_IMAGE (8G)..."
qemu-img create -f qcow2 "$DISK_IMAGE" 8G

echo "[create-openrc-vm] connecting via qemu-nbd..."
qemu-nbd --connect="$NBD_DEV" "$DISK_IMAGE"
sleep 2

echo "[create-openrc-vm] partitioning (single ext4 root)..."
parted -s "$NBD_DEV" mklabel msdos mkpart primary ext4 1MiB 100%
sleep 1

echo "[create-openrc-vm] formatting /dev/nbd0p1 as ext4..."
mkfs.ext4 -F "${NBD_DEV}p1"

echo "[create-openrc-vm] mounting..."
mount "${NBD_DEV}p1" "$MOUNT_DIR"

echo "[create-openrc-vm] running pacstrap (base linux elogind-openrc iptables openssh syslog-ng networkmanager networkmanager-openrc)..."
# linux: provides /lib/modules matching the host kernel — udev needs this for coldplug.
# networkmanager-openrc: provides /etc/init.d/NetworkManager for OpenRC.
# Explicit elogind-openrc and iptables to avoid interactive provider prompts.
pacstrap -K "$MOUNT_DIR" base linux elogind-openrc iptables openssh syslog-ng networkmanager networkmanager-openrc

echo "[create-openrc-vm] writing /etc/fstab..."
cat > "$MOUNT_DIR/etc/fstab" << 'EOF'
/dev/vda1   /     ext4   defaults,relatime   0  1
tmpfs       /tmp  tmpfs  defaults            0  0
EOF

echo "[create-openrc-vm] setting hostname..."
echo "artix-openrc" > "$MOUNT_DIR/etc/hostname"

echo "[create-openrc-vm] removing root password (testing only)..."
arch-chroot "$MOUNT_DIR" passwd -d root

echo "[create-openrc-vm] generating ssh host keys..."
arch-chroot "$MOUNT_DIR" ssh-keygen -A

# ---- Configure serial console getty ----
# OpenRC uses inittab for getty. The default has ttyS0 commented out.
# Add/enable ttyS0 so the login prompt appears on the serial console.
echo "[create-openrc-vm] configuring ttyS0 serial getty..."
INITTAB="$MOUNT_DIR/etc/inittab"
if [ -f "$INITTAB" ]; then
    # Uncomment ttyS0 if present
    sed -i 's|^#\(.*ttyS0.*agetty.*\)|\1|' "$INITTAB"
    # Add if not present at all
    if ! grep -q 'ttyS0' "$INITTAB"; then
        echo 'ttyS0::respawn:/sbin/agetty -L ttyS0 115200 vt100' >> "$INITTAB"
    fi
    echo "[create-openrc-vm] inittab ttyS0 configured"
else
    # Create a minimal inittab
    cat > "$INITTAB" << 'INITTAB_EOF'
# /etc/inittab
::sysinit:/sbin/openrc sysinit
::sysinit:/sbin/openrc boot
::wait:/sbin/openrc default

tty1::respawn:/sbin/agetty 38400 tty1 linux
ttyS0::respawn:/sbin/agetty -L ttyS0 115200 vt100

::ctrlaltdel:/sbin/reboot
::shutdown:/sbin/openrc shutdown
INITTAB_EOF
    echo "[create-openrc-vm] /etc/inittab created"
fi

# ---- Enable services in OpenRC runlevels ----
echo "[create-openrc-vm] enabling services in default runlevel..."
for svc in syslog-ng NetworkManager sshd; do
    if [ -f "$MOUNT_DIR/etc/init.d/$svc" ]; then
        ln -sf "/etc/init.d/$svc" "$MOUNT_DIR/etc/runlevels/default/$svc" 2>/dev/null || true
        echo "[create-openrc-vm]   enabled: $svc"
    fi
done

# Remove the broken generic agetty symlink that causes "cannot be started directly" error
rm -f "$MOUNT_DIR/etc/runlevels/default/agetty"
echo "[create-openrc-vm] removed spurious generic agetty from default runlevel"

# Remove netmount — it depends on 'net' (netifrc interface services) which we don't have.
# On a minimal VM without NFS, netmount serves no purpose and causes a dependency error.
rm -f "$MOUNT_DIR/etc/runlevels/default/netmount"
echo "[create-openrc-vm] removed netmount from default runlevel (no net.ethX configured)"

# Enable rc_parallel so OpenRC starts independent services simultaneously —
# this is the recommended setting for modern hardware and a fair comparison.
sed -i 's|^#rc_parallel="NO"|rc_parallel="YES"|' "$MOUNT_DIR/etc/rc.conf"
echo "[create-openrc-vm] enabled rc_parallel"

sync
echo "[create-openrc-vm] done. Image: $DISK_IMAGE"
echo "[create-openrc-vm] measure with: sudo bash scripts/measure-openrc-installed.sh $DISK_IMAGE"
