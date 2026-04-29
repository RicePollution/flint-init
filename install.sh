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

if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
    main "$@"
fi
