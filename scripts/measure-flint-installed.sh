#!/usr/bin/env bash
# scripts/measure-flint-installed.sh — measure flint-init boot time on an
# installed Artix disk image, averaged over N runs.
#
# Methodology (matches measure-openrc-installed.sh):
#   - Same host kernel (-kernel), no initramfs
#   - Same virtio disk, same RAM/CPU
#   - Measure: QEMU launch → "login:" on ttyS0
#
# Usage: bash scripts/measure-flint-installed.sh [artix-openrc.qcow2] [runs]

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DISK_IMAGE="${1:-$REPO_ROOT/artix-openrc.qcow2}"
RUNS="${2:-5}"
KERNEL="/boot/vmlinuz-linux"
SERIAL_LOG="/tmp/flint-measure-serial.txt"
TIMEOUT=60

if [ ! -f "$DISK_IMAGE" ]; then
    echo "error: disk image not found: $DISK_IMAGE" >&2
    exit 1
fi

echo "[measure] disk:   $DISK_IMAGE"
echo "[measure] kernel: $KERNEL"
echo "[measure] runs:   $RUNS"
echo ""

TIMES=()

for run in $(seq 1 "$RUNS"); do
    rm -f "$SERIAL_LOG"
    touch "$SERIAL_LOG"

    T0_NS=$(date +%s%N)

    qemu-system-x86_64 \
        -enable-kvm \
        -drive "file=$DISK_IMAGE,if=virtio,format=qcow2,snapshot=on" \
        -kernel "$KERNEL" \
        -append "root=/dev/vda1 rw init=/usr/sbin/flint-init console=ttyS0 loglevel=7" \
        -display none \
        -chardev "file,id=char0,path=$SERIAL_LOG" \
        -serial chardev:char0 \
        -m 512M \
        -smp 2 \
        -netdev user,id=net0 \
        -device virtio-net-pci,netdev=net0 \
        -no-reboot &

    QEMU_PID=$!
    LOGIN_NS=""
    START_WAIT=$SECONDS

    while [ $(( SECONDS - START_WAIT )) -lt $TIMEOUT ]; do
        if ! kill -0 "$QEMU_PID" 2>/dev/null; then
            break
        fi
        if grep -qE 'login:' "$SERIAL_LOG" 2>/dev/null; then
            LOGIN_NS=$(date +%s%N)
            break
        fi
        sleep 0.05
    done

    kill "$QEMU_PID" 2>/dev/null || true
    wait "$QEMU_PID" 2>/dev/null || true

    if [ -z "$LOGIN_NS" ]; then
        echo "[measure] run $run: TIMEOUT — login prompt not seen within ${TIMEOUT}s" >&2
        continue
    fi

    # Wall-clock from QEMU start to login prompt
    WALL_MS=$(( ( LOGIN_NS - T0_NS ) / 1000000 ))

    # Kernel timestamp when flint-init was exec'd (seconds, from dmesg in serial log)
    # Subtract this to get pure userspace time
    FLINT_EXEC_S=$(grep -oP '^\[\s*\K[0-9]+\.[0-9]+(?=\] Run /usr/sbin/flint-init)' "$SERIAL_LOG" | head -1)

    if [ -n "$FLINT_EXEC_S" ]; then
        KERNEL_MS=$(awk "BEGIN { printf \"%d\", $FLINT_EXEC_S * 1000 }")
        MS=$(( WALL_MS - KERNEL_MS ))
    else
        MS=$WALL_MS
    fi

    TIMES+=("$MS")
    echo "[measure] run $run: ${MS} ms userspace  (${WALL_MS} ms total wall)"
done

# ---- Stats ----
N=${#TIMES[@]}
if [ "$N" -eq 0 ]; then
    echo "error: no successful runs" >&2
    exit 1
fi

SUM=0
MIN=${TIMES[0]}
MAX=${TIMES[0]}
for t in "${TIMES[@]}"; do
    SUM=$(( SUM + t ))
    [ "$t" -lt "$MIN" ] && MIN=$t
    [ "$t" -gt "$MAX" ] && MAX=$t
done
MEAN=$(( SUM / N ))

KERNEL_VER=$(uname -r)

echo ""
echo "========================================"
echo "  flint-init boot measurement"
echo "========================================"
echo "  runs:    $N"
echo "  kernel:  $KERNEL_VER"
echo "  mean:    ${MEAN} ms"
echo "  min:     ${MIN} ms"
echo "  max:     ${MAX} ms"
echo "========================================"
echo ""
echo "README table row:"
printf "| flint-init (Artix, installed) | %s | — | **%s ms** | %s ms |\n" \
    "$KERNEL_VER" "$MEAN" "$MAX"
