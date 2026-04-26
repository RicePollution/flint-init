#!/usr/bin/env bash
# scripts/measure-systemd-boot.sh
# Downloads Arch Linux live ISO, boots it in QEMU, measures systemd boot timestamps.
#
# Outputs: kernel boot time, userspace (PID 1 → login) time, total.
# Same methodology as the OpenRC/flint-init comparison in the README.
#
# Requirements: qemu-system-x86_64, curl, bsdtar or 7z, file
# No root required.

set -euo pipefail

ISO_URL="https://geo.mirror.pkgbuild.com/iso/latest/archlinux-x86_64.iso"
ISO_PATH="/tmp/archlinux-x86_64.iso"
EXTRACT_DIR="/tmp/arch-iso-extract"
SERIAL_LOG="/tmp/arch-systemd-serial.log"
TIMEOUT=300  # seconds to wait for login prompt

# ---- 1. Download ISO ----
if [ ! -f "$ISO_PATH" ]; then
    echo "[measure] Downloading Arch Linux ISO (~900 MB)..."
    curl -L --progress-bar -o "${ISO_PATH}.tmp" "$ISO_URL"
    mv "${ISO_PATH}.tmp" "$ISO_PATH"
else
    echo "[measure] Using cached ISO: $ISO_PATH ($(du -sh "$ISO_PATH" | cut -f1))"
fi

# ---- 2. Get ISO label ----
ISO_LABEL=$(file "$ISO_PATH" 2>/dev/null | grep -oP "(?<=')[A-Z_0-9]+" | head -1 || true)
if [ -z "$ISO_LABEL" ]; then
    # Fallback: read label from ISO header at offset 32808
    ISO_LABEL=$(dd if="$ISO_PATH" bs=1 skip=32808 count=32 2>/dev/null | tr -d ' \0')
fi
echo "[measure] ISO label: $ISO_LABEL"

# ---- 3. Extract kernel + initramfs ----
mkdir -p "$EXTRACT_DIR"

VMLINUZ="$EXTRACT_DIR/arch/boot/x86_64/vmlinuz-linux"
INITRAMFS="$EXTRACT_DIR/arch/boot/x86_64/initramfs-linux.img"

if [ ! -f "$VMLINUZ" ] || [ ! -f "$INITRAMFS" ]; then
    echo "[measure] Extracting kernel and initramfs from ISO..."
    if command -v bsdtar &>/dev/null; then
        bsdtar -xf "$ISO_PATH" -C "$EXTRACT_DIR" \
            arch/boot/x86_64/vmlinuz-linux \
            arch/boot/x86_64/initramfs-linux.img 2>/dev/null
    elif command -v 7z &>/dev/null; then
        7z x "$ISO_PATH" -o"$EXTRACT_DIR" \
            arch/boot/x86_64/vmlinuz-linux \
            arch/boot/x86_64/initramfs-linux.img >/dev/null
    else
        echo "error: need bsdtar or 7z to extract from ISO"
        exit 1
    fi
fi

if [ ! -f "$VMLINUZ" ] || [ ! -f "$INITRAMFS" ]; then
    echo "error: failed to extract kernel/initramfs from ISO"
    ls -la "$EXTRACT_DIR/arch/boot/x86_64/" 2>/dev/null || true
    exit 1
fi

echo "[measure] kernel:    $VMLINUZ ($(du -sh "$VMLINUZ" | cut -f1))"
echo "[measure] initramfs: $INITRAMFS ($(du -sh "$INITRAMFS" | cut -f1))"

# ---- 4. Boot in QEMU ----
rm -f "$SERIAL_LOG"
touch "$SERIAL_LOG"

echo "[measure] Starting QEMU..."
T0_NS=$(date +%s%N)

qemu-system-x86_64 \
    -kernel "$VMLINUZ" \
    -initrd "$INITRAMFS" \
    -append "archisobasedir=arch archisolabel=${ISO_LABEL} console=ttyS0 loglevel=5 systemd.show_status=1 systemd.mask=systemd-networkd-wait-online.service" \
    -drive "file=$ISO_PATH,media=cdrom,readonly=on,if=ide" \
    -serial "file:$SERIAL_LOG" \
    -display none \
    -m 1G \
    -smp 2 \
    -no-reboot &

QEMU_PID=$!

# ---- 5. Poll serial log for timestamps ----
echo "[measure] Waiting for boot (timeout: ${TIMEOUT}s)..."
echo "[measure] Serial log: $SERIAL_LOG"

PID1_NS=""
LOGIN_NS=""

START_WAIT=$SECONDS

while [ $(( SECONDS - START_WAIT )) -lt $TIMEOUT ]; do
    if ! kill -0 "$QEMU_PID" 2>/dev/null; then
        echo "[measure] QEMU exited early"
        break
    fi

    if [ -z "$PID1_NS" ] && grep -q 'systemd\[1\]' "$SERIAL_LOG" 2>/dev/null; then
        PID1_NS=$(date +%s%N)
        echo "[measure] systemd[1] seen at $(( ( PID1_NS - T0_NS ) / 1000000 )) ms from QEMU start"
    fi

    if [ -z "$LOGIN_NS" ] && grep -qE '(archiso login:|Arch Linux.*\(ttyS0\)|login: $)' "$SERIAL_LOG" 2>/dev/null; then
        LOGIN_NS=$(date +%s%N)
        echo "[measure] login prompt seen at $(( ( LOGIN_NS - T0_NS ) / 1000000 )) ms from QEMU start"
        break
    fi

    sleep 0.1
done

# Kill QEMU
kill "$QEMU_PID" 2>/dev/null || true
wait "$QEMU_PID" 2>/dev/null || true

# ---- 6. Print results ----
echo ""

if [ -z "$PID1_NS" ] || [ -z "$LOGIN_NS" ]; then
    echo "[measure] ERROR: did not capture all timestamps within ${TIMEOUT}s"
    echo "[measure] PID1 captured: $([ -n "$PID1_NS" ] && echo yes || echo NO)"
    echo "[measure] Login captured: $([ -n "$LOGIN_NS" ] && echo yes || echo NO)"
    echo ""
    echo "[measure] Last 80 lines of serial log:"
    tail -80 "$SERIAL_LOG"
    exit 1
fi

KERNEL_MS=$(( ( PID1_NS - T0_NS ) / 1000000 ))
USERSPACE_MS=$(( ( LOGIN_NS - PID1_NS ) / 1000000 ))
TOTAL_MS=$(( ( LOGIN_NS - T0_NS ) / 1000000 ))

SYSTEMD_VER=$(grep -oP 'systemd \K[0-9]+' "$SERIAL_LOG" | head -1 || echo "???")
KERNEL_VER=$(grep -oP 'Linux version \K[0-9]+\.[0-9]+[^ ]*' "$SERIAL_LOG" | head -1 || echo "???")

echo "========================================"
echo "  systemd boot measurement"
echo "========================================"
echo "  systemd version:           $SYSTEMD_VER"
echo "  kernel:                    $KERNEL_VER"
echo "  kernel boot (QEMU → PID1): ${KERNEL_MS} ms"
echo "  userspace (PID 1 → login): ${USERSPACE_MS} ms"
echo "  total:                     ${TOTAL_MS} ms"
echo "========================================"
echo ""
echo "README table row:"
printf "| systemd %s (Arch live ISO) | %s | %s ms | **%s ms** | %s ms |\n" \
    "$SYSTEMD_VER" "$KERNEL_VER" "$KERNEL_MS" "$USERSPACE_MS" "$TOTAL_MS"
