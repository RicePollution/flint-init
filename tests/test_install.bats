setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
    source "$REPO_ROOT/install.sh"
    TMPROOT="$(mktemp -d)"
    ROOT="$TMPROOT"
}

teardown() {
    rm -rf "$TMPROOT"
}
