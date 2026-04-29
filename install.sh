#!/usr/bin/env bash
# install.sh — install flint-init onto an Artix Linux system
#
# Usage:
#   sudo bash install.sh                    # install on running system
#   sudo bash install.sh --root /mnt/artix  # install into mounted root
#   sudo bash install.sh --build            # force build from source
#   sudo bash install.sh --download         # force download from GitHub Releases

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="/"
ACQUIRE_MODE="auto"  # auto | build | download
DISTRO=""
FLINT_BIN=""
FLINT_CTL_BIN=""
_ACQUIRE_TMPDIR=""

# Overridable for tests
_FLINT_OS_RELEASE="${_FLINT_OS_RELEASE:-/etc/os-release}"

check_root() {
    if [ "$(id -u)" -ne 0 ]; then
        echo "[flint-install] error: must run as root" >&2
        exit 1
    fi
}

detect_distro() {
    if [ ! -f "$_FLINT_OS_RELEASE" ]; then
        echo "[flint-install] error: $_FLINT_OS_RELEASE not found" >&2
        exit 1
    fi
    # shellcheck source=/dev/null
    source "$_FLINT_OS_RELEASE"
    local id="${ID:-}"
    local id_like="${ID_LIKE:-}"
    case "$id" in
        artix) DISTRO=artix ;;
        arch)  DISTRO=arch ;;
        void)  DISTRO=void ;;
        *)
            case "$id_like" in
                *arch*) DISTRO=arch ;;
                *void*) DISTRO=void ;;
                *)
                    echo "[flint-install] error: unsupported distro: ${id:-unknown}" >&2
                    echo "[flint-install] supported distros: artix, arch, void" >&2
                    exit 1
                    ;;
            esac
            ;;
    esac
    echo "[flint-install] detected distro: $DISTRO [ok]"
}

_download_release() {
    local tmpdir="$1"
    local arch="x86_64"
    local url="https://github.com/RicePollution/flint-init/releases/latest/download/flint-init-${arch}-linux.tar.gz"
    echo "[flint-install] downloading pre-built release..."
    if ! curl -fsSL "$url" -o "$tmpdir/flint-init.tar.gz"; then
        echo "[flint-install] download failed" >&2
        return 1
    fi
    tar -xzf "$tmpdir/flint-init.tar.gz" -C "$tmpdir"
    FLINT_BIN="$tmpdir/flint-init"
    FLINT_CTL_BIN="$tmpdir/flint-ctl"
    echo "[flint-install] downloaded release [ok]"
}

_build_from_source() {
    echo "[flint-install] building from source..."
    if ! command -v cargo &>/dev/null; then
        echo "[flint-install] error: cargo not found" >&2
        echo "[flint-install] options:" >&2
        echo "[flint-install]   install Rust: https://rustup.rs" >&2
        echo "[flint-install]   or ensure a GitHub release exists to download" >&2
        exit 1
    fi
    cargo build --release --manifest-path "$REPO_ROOT/Cargo.toml"
    FLINT_BIN="$REPO_ROOT/target/release/flint-init"
    FLINT_CTL_BIN="$REPO_ROOT/target/release/flint-ctl"
    echo "[flint-install] build complete [ok]"
}

acquire_binary() {
    local tmpdir
    tmpdir="$(mktemp -d)"
    _ACQUIRE_TMPDIR="$tmpdir"

    case "$ACQUIRE_MODE" in
        download)
            _download_release "$tmpdir" || {
                echo "[flint-install] error: download failed and --download was forced" >&2
                exit 1
            }
            ;;
        build)
            _build_from_source
            ;;
        auto)
            _download_release "$tmpdir" || _build_from_source
            ;;
    esac
}

if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
    main "$@"
fi
