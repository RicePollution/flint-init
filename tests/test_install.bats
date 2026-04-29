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
