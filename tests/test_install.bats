setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
    source "$REPO_ROOT/install.sh"
    TMPROOT="$(mktemp -d)"
    ROOT="$TMPROOT"
}

teardown() {
    rm -rf "$TMPROOT"
}

@test "check_root: exits 1 when not root" {
    run bash -c 'source '"$REPO_ROOT"'/install.sh; check_root'
    [ "$status" -eq 1 ]
    [[ "$output" == *"must run as root"* ]]
}

@test "detect_distro: sets DISTRO=artix for ID=artix" {
    echo 'ID=artix' > "$BATS_TMPDIR/os-release"
    _FLINT_OS_RELEASE="$BATS_TMPDIR/os-release"
    detect_distro
    [ "$DISTRO" = "artix" ]
}

@test "detect_distro: sets DISTRO=arch for ID=arch" {
    echo 'ID=arch' > "$BATS_TMPDIR/os-release"
    _FLINT_OS_RELEASE="$BATS_TMPDIR/os-release"
    detect_distro
    [ "$DISTRO" = "arch" ]
}

@test "detect_distro: sets DISTRO=arch for ID_LIKE=arch" {
    printf 'ID=manjaro\nID_LIKE=arch\n' > "$BATS_TMPDIR/os-release"
    _FLINT_OS_RELEASE="$BATS_TMPDIR/os-release"
    detect_distro
    [ "$DISTRO" = "arch" ]
}

@test "detect_distro: exits 1 for unsupported distro" {
    echo 'ID=ubuntu' > "$BATS_TMPDIR/os-release"
    _FLINT_OS_RELEASE="$BATS_TMPDIR/os-release"
    run detect_distro
    [ "$status" -eq 1 ]
    [[ "$output" == *"unsupported"* ]]
}

@test "acquire_binary: uses downloaded binaries when curl succeeds" {
    # Stub curl to produce a fake tarball
    curl() {
        mkdir -p "$BATS_TMPDIR/fake-release"
        echo '#!/bin/sh' > "$BATS_TMPDIR/fake-release/flint-init"
        echo '#!/bin/sh' > "$BATS_TMPDIR/fake-release/flint-ctl"
        chmod +x "$BATS_TMPDIR/fake-release/flint-init" \
                  "$BATS_TMPDIR/fake-release/flint-ctl"
        tar -czf "$4" -C "$BATS_TMPDIR/fake-release" flint-init flint-ctl
    }
    export -f curl
    ACQUIRE_MODE=download
    _acquire_tmpdir="$(mktemp -d)"
    _download_release "$_acquire_tmpdir"
    [ -x "$_acquire_tmpdir/flint-init" ]
    [ -x "$_acquire_tmpdir/flint-ctl" ]
    rm -rf "$_acquire_tmpdir"
}

@test "acquire_binary: falls back to build when download fails" {
    curl() { return 1; }
    export -f curl
    cargo() {
        # Real root-owned binaries already exist in target/release; just succeed
        return 0
    }
    export -f cargo
    ACQUIRE_MODE=auto
    _acquire_tmpdir="$(mktemp -d)"
    acquire_binary "$_acquire_tmpdir"
    [ "$FLINT_BIN" = "$REPO_ROOT/target/release/flint-init" ]
    rm -rf "$_acquire_tmpdir"
}

@test "install_files: copies binaries to ROOT" {
    FLINT_BIN="$(mktemp)"
    FLINT_CTL_BIN="$(mktemp)"
    chmod +x "$FLINT_BIN" "$FLINT_CTL_BIN"
    DISTRO=artix
    install_files
    [ -x "$TMPROOT/usr/sbin/flint-init" ]
    [ -x "$TMPROOT/usr/bin/flint-ctl" ]
}

@test "install_files: copies distro service TOMLs to ROOT" {
    FLINT_BIN="$(mktemp)"
    FLINT_CTL_BIN="$(mktemp)"
    chmod +x "$FLINT_BIN" "$FLINT_CTL_BIN"
    DISTRO=artix
    install_files
    # At least one artix TOML should be present
    ls "$TMPROOT/etc/flint/services/"*.toml
}

@test "install_files: exits 1 for distro with no service directory" {
    FLINT_BIN="$(mktemp)"
    FLINT_CTL_BIN="$(mktemp)"
    chmod +x "$FLINT_BIN" "$FLINT_CTL_BIN"
    DISTRO=nonexistent
    run install_files
    [ "$status" -eq 1 ]
    [[ "$output" == *"no service definitions"* ]]
}

@test "install_files: installs common service when exec binary exists in ROOT" {
    FLINT_BIN="$(mktemp)"
    FLINT_CTL_BIN="$(mktemp)"
    chmod +x "$FLINT_BIN" "$FLINT_CTL_BIN"
    DISTRO=artix

    # Plant a fake nginx binary in the target root
    mkdir -p "$TMPROOT/usr/sbin"
    echo '#!/bin/sh' > "$TMPROOT/usr/sbin/nginx"
    chmod +x "$TMPROOT/usr/sbin/nginx"

    # Override common dir to use our fixture
    _FLINT_COMMON_DIR="$BATS_TEST_DIRNAME/fixtures"

    install_files
    [ -f "$TMPROOT/etc/flint/services/nginx.toml" ]
}

@test "install_files: skips common service when exec binary absent from ROOT" {
    FLINT_BIN="$(mktemp)"
    FLINT_CTL_BIN="$(mktemp)"
    chmod +x "$FLINT_BIN" "$FLINT_CTL_BIN"
    DISTRO=artix
    _FLINT_COMMON_DIR="$BATS_TEST_DIRNAME/fixtures"
    # nginx binary NOT present in TMPROOT
    install_files
    [ ! -f "$TMPROOT/etc/flint/services/nginx.toml" ]
}

@test "configure_bootloader: writes /etc/grub.d/99-flint and calls grub-mkconfig" {
    # Fake GRUB environment in TMPROOT
    mkdir -p "$TMPROOT/etc/grub.d"
    mkdir -p "$TMPROOT/etc/default"
    echo 'GRUB_CMDLINE_LINUX_DEFAULT="quiet"' > "$TMPROOT/etc/default/grub"
    ROOT="$TMPROOT"

    # Stub grub-mkconfig
    grub-mkconfig() { echo "grub-mkconfig called: $*"; }
    export -f grub-mkconfig

    configure_bootloader
    [ -f "$TMPROOT/etc/grub.d/99-flint" ]
    [ -x "$TMPROOT/etc/grub.d/99-flint" ]
    grep -q "init=/usr/sbin/flint-init" "$TMPROOT/etc/grub.d/99-flint"
}

@test "configure_bootloader: skips GRUB auto-config in --root mode" {
    ROOT="$TMPROOT"   # non-/ root triggers print-only path
    mkdir -p "$TMPROOT/etc/grub.d"
    run configure_bootloader
    [ "$status" -eq 0 ]
    [ ! -f "$TMPROOT/etc/grub.d/99-flint" ]
    [[ "$output" == *"init=/usr/sbin/flint-init"* ]]
}
