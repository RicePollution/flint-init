#!/usr/bin/env bash
# scripts/qemu-stage3-test.sh — Stage 3 integration test in QEMU.
#
# Tests:
#   1. Restart logic: restart-svc fails twice then succeeds; verify 3 attempts logged
#   2. flint-ctl status: ctl-test queries the running supervisor via /run/flint/ctl.sock
#
# The VM halts automatically once all test services complete (FLINT_ON_EXIT=halt).
# A 60s timeout kills QEMU if something hangs.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
INITRAMFS_DIR="$(mktemp -d /tmp/flint-stage3-XXXXXX)"
OUTPUT_CPIO="/tmp/flint-stage3.cpio.gz"
KERNEL="/boot/vmlinuz-linux"

cleanup() { rm -rf "$INITRAMFS_DIR"; }
trap cleanup EXIT

echo "[stage3-test] building flint-init and flint-ctl (release)..."
cargo build --release --manifest-path "$REPO_ROOT/Cargo.toml"

echo "[stage3-test] scaffolding initramfs at $INITRAMFS_DIR..."
mkdir -p "$INITRAMFS_DIR"/{proc,sys,dev,run,tmp,bin,usr/bin,usr/lib,services/test}
ln -sf usr/lib "$INITRAMFS_DIR/lib"
ln -sf usr/lib "$INITRAMFS_DIR/lib64"
ln -sf lib     "$INITRAMFS_DIR/usr/lib64"

# copy binary + all its ldd-reported shared libs into the initramfs
copy_bin() {
    local src="$1" dst="$2"
    install -D -m 755 "$src" "$dst"
    ldd "$src" 2>/dev/null | awk '{
        if ($2 == "=>") { if ($3 != "not") print $3 }
        else if ($1 ~ /^\//) print $1
    }' | while read -r lib; do
        [ -f "$lib" ] || continue
        local dest="$INITRAMFS_DIR$lib"
        mkdir -p "$(dirname "$dest")"
        [ -f "$dest" ] || cp -L "$lib" "$dest"
    done
}

copy_bin "$REPO_ROOT/target/release/flint-init" "$INITRAMFS_DIR/init"
copy_bin "$REPO_ROOT/target/release/flint-ctl"  "$INITRAMFS_DIR/usr/bin/flint-ctl"
copy_bin /bin/sh                                  "$INITRAMFS_DIR/bin/sh"
copy_bin /usr/bin/sleep                           "$INITRAMFS_DIR/usr/bin/sleep"

# ---- test service: a ----
# Exits 0 immediately. Verifies basic startup is unaffected.
cat > "$INITRAMFS_DIR/services/test/a.toml" << 'EOF'
[service]
name = "a"
exec = "/bin/sh /usr/lib/svc-a.sh"
restart = "never"
EOF
cat > "$INITRAMFS_DIR/usr/lib/svc-a.sh" << 'EOF'
#!/bin/sh
echo "[svc-a] started, exiting 0"
exit 0
EOF
chmod 755 "$INITRAMFS_DIR/usr/lib/svc-a.sh"

# ---- test service: restart-svc ----
# Fails on attempts 1 and 2 (exit 1), succeeds on attempt 3 (exit 0).
# restart = "on-failure" — expects exactly 2 restarts then completion.
cat > "$INITRAMFS_DIR/services/test/restart-svc.toml" << 'EOF'
[service]
name = "restart-svc"
exec = "/bin/sh /usr/lib/restart-test.sh"
restart = "on-failure"

[deps]
after = ["a"]
EOF
cat > "$INITRAMFS_DIR/usr/lib/restart-test.sh" << 'EOF'
#!/bin/sh
COUNT_FILE="/run/restart-count"
COUNT=0
[ -f "$COUNT_FILE" ] && read -r COUNT < "$COUNT_FILE"
COUNT=$((COUNT + 1))
printf '%s\n' "$COUNT" > "$COUNT_FILE"
if [ "$COUNT" -lt 3 ]; then
    echo "[restart-test] attempt $COUNT: FAILING (exit 1)"
    exit 1
fi
echo "[restart-test] attempt $COUNT: SUCCESS (exit 0)"
exit 0
EOF
chmod 755 "$INITRAMFS_DIR/usr/lib/restart-test.sh"

# ---- test service: ctl-test ----
# Depends on restart-svc succeeding (via needs).
# Queries flint-ctl status and prints the JSON output.
# When this exits 0, all services are done and the VM halts.
cat > "$INITRAMFS_DIR/services/test/ctl-test.toml" << 'EOF'
[service]
name = "ctl-test"
exec = "/bin/sh /usr/lib/ctl-test.sh"
restart = "never"

[deps]
needs = ["restart-svc"]
EOF
cat > "$INITRAMFS_DIR/usr/lib/ctl-test.sh" << 'EOF'
#!/bin/sh
# Give the ctl server thread a moment to bind the socket.
# (restart-svc takes ~100ms+ to retry, so the socket is usually ready,
# but sleep adds safety margin.)
/usr/bin/sleep 0.3
echo "[ctl-test] --- querying flint-ctl status ---"
/usr/bin/flint-ctl status && echo "[ctl-test] flint-ctl: OK" || echo "[ctl-test] flint-ctl: FAILED"
echo "[ctl-test] --- done ---"
exit 0
EOF
chmod 755 "$INITRAMFS_DIR/usr/lib/ctl-test.sh"

echo "[stage3-test] packing initramfs..."
(cd "$INITRAMFS_DIR" && find . | cpio -o -H newc 2>/dev/null | gzip -9 > "$OUTPUT_CPIO")
echo "[stage3-test] initramfs size: $(du -sh "$OUTPUT_CPIO" | cut -f1)"

SERIAL_LOG="/tmp/flint-stage3-serial.txt"
echo "[stage3-test] booting QEMU (60s timeout)..."
timeout 60 qemu-system-x86_64 \
    -kernel "$KERNEL" \
    -initrd "$OUTPUT_CPIO" \
    -append "init=/init FLINT_ON_EXIT=halt console=ttyS0 earlyprintk=serial,ttyS0,115200 -- /services/test" \
    -display none \
    -chardev "file,id=char0,path=$SERIAL_LOG" \
    -serial chardev:char0 \
    -m 256M \
    -no-reboot 2>/dev/null
echo "--- VM output ---"
cat "$SERIAL_LOG"
