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
