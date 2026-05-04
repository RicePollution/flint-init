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
_FLINT_COMMON_DIR="${_FLINT_COMMON_DIR:-$REPO_ROOT/services/global}"

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

install_files() {
    echo "[flint-install] installing binaries..."
    install -D -m 755 "$FLINT_BIN"     "$ROOT/usr/sbin/flint-init"
    install -D -m 755 "$FLINT_CTL_BIN" "$ROOT/usr/bin/flint-ctl"
    echo "[flint-install]   $ROOT/usr/sbin/flint-init [ok]"
    echo "[flint-install]   $ROOT/usr/bin/flint-ctl [ok]"

    echo "[flint-install] installing reboot wrapper..."
    rm -f "$ROOT/usr/sbin/reboot"
    cat > "$ROOT/usr/sbin/reboot" << 'REBOOT_SCRIPT'
#!/bin/sh
# If flint-init is PID 1, send SIGUSR1 for graceful reboot.
# Otherwise fall through to the real reboot binary.
if readlink /proc/1/exe 2>/dev/null | grep -q flint-init; then
    kill -USR1 1
else
    exec /usr/bin/reboot "$@"
fi
REBOOT_SCRIPT
    chmod +x "$ROOT/usr/sbin/reboot"
    echo "[flint-install]   $ROOT/usr/sbin/reboot [ok]"

    local svc_dir="$REPO_ROOT/services/$DISTRO"
    if [ ! -d "$svc_dir" ]; then
        echo "[flint-install] error: no service definitions for $DISTRO at $svc_dir" >&2
        exit 1
    fi

    echo "[flint-install] installing $DISTRO service definitions..."
    mkdir -p "$ROOT/etc/flint/services"
    cp "$svc_dir"/*.toml "$ROOT/etc/flint/services/"
    echo "[flint-install]   $(ls "$svc_dir"/*.toml | wc -l) services installed [ok]"

    if [ -d "$_FLINT_COMMON_DIR" ]; then
        local installed_common=0
        for toml in "$_FLINT_COMMON_DIR"/*.toml; do
            [ -f "$toml" ] || continue
            # Extract first word of exec value: exec = "/path/to/bin args..."
            local exec_line exec_full exec_bin
            exec_line=$(grep -E '^\s*exec\s*=' "$toml" | head -1)
            exec_full=$(echo "$exec_line" | sed 's/.*=\s*"\([^"]*\)".*/\1/')
            exec_bin=$(echo "$exec_full" | awk '{print $1}')
            if [ -n "$exec_bin" ] && [ -x "$ROOT$exec_bin" ]; then
                cp "$toml" "$ROOT/etc/flint/services/"
                echo "[flint-install]   + $(basename "$toml") (common)"
                installed_common=$((installed_common + 1))
            fi
        done
        echo "[flint-install]   $installed_common common services installed [ok]"
    fi
}

_configure_grub() {
    echo "[flint-install] configuring GRUB..."
    local entry_file="$ROOT/etc/grub.d/99-flint"
    local cmdline kernel initrd root_uuid
    cmdline=$(grep 'GRUB_CMDLINE_LINUX_DEFAULT' "$ROOT/etc/default/grub" \
              | sed 's/.*="\(.*\)"/\1/')
    kernel=$(ls "${ROOT}/boot/vmlinuz-linux" 2>/dev/null | head -1 | sed "s|^${ROOT}||")
    initrd=$(ls "${ROOT}/boot/initramfs-linux.img" 2>/dev/null | head -1 | sed "s|^${ROOT}||")
    root_uuid=$(findmnt -n -o UUID "${ROOT:-/}" 2>/dev/null)

    # Write a grub.d shell script — grub-mkconfig runs it and captures stdout
    {
        echo '#!/bin/sh'
        echo 'cat << '"'"'GRUB_EOF'"'"
        echo "menuentry 'Linux, with flint-init' {"
        echo "    search --no-floppy --fs-uuid --set=root ${root_uuid}"
        echo "    linux   ${kernel} root=UUID=${root_uuid} rw ${cmdline} init=/usr/sbin/flint-init"
        echo "    initrd  ${initrd}"
        echo "}"
        echo 'GRUB_EOF'
    } > "$entry_file"
    chmod +x "$entry_file"
    grub-mkconfig -o "${ROOT}/boot/grub/grub.cfg"
    echo "[flint-install] GRUB entry written [ok]"
}

_print_bootloader_instructions() {
    cat << 'INSTRUCTIONS'

==========================================
  Bootloader configuration
==========================================

Add this to your kernel parameters:

  init=/usr/sbin/flint-init

systemd-boot:
  Edit /boot/loader/entries/<your-entry>.conf
  Append 'init=/usr/sbin/flint-init' to the 'options' line

rEFInd:
  Edit /boot/refind_linux.conf
  Append 'init=/usr/sbin/flint-init' to your boot stanza options line

GRUB (manual):
  Edit /etc/default/grub — add to GRUB_CMDLINE_LINUX
  Then run: grub-mkconfig -o /boot/grub/grub.cfg

INSTRUCTIONS
}

configure_bootloader() {
    if [ -f "$ROOT/etc/default/grub" ] && command -v grub-mkconfig &>/dev/null; then
        _configure_grub || {
            echo "[flint-install] warning: GRUB auto-config failed, see instructions below" >&2
            _print_bootloader_instructions
        }
    else
        _print_bootloader_instructions
    fi
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

SESSION_USER=""

detect_session_user() {
    # Prefer $SUDO_USER — set when the script is run via sudo
    if [ -n "${SUDO_USER:-}" ] && [ "$SUDO_USER" != "root" ]; then
        SESSION_USER="$SUDO_USER"
        return
    fi
    # Fall back to logname (the user who originally opened the terminal)
    local logged_in
    logged_in=$(logname 2>/dev/null) || true
    if [ -n "$logged_in" ] && [ "$logged_in" != "root" ]; then
        SESSION_USER="$logged_in"
        return
    fi
    # Last resort: prompt
    echo ""
    echo "[flint-install] Desktop services (hyprland, waybar, pipewire, etc.) need a session user."
    read -r -p "[flint-install] Enter your login username (leave empty to skip): " SESSION_USER || true
}

write_flint_config() {
    detect_session_user
    mkdir -p "$ROOT/etc/flint"
    if [ -z "$SESSION_USER" ]; then
        echo "[flint-install] note: no session user set; edit /etc/flint/config.toml to configure desktop services" >&2
        # Still write the config file with an empty placeholder so the path exists
        cat > "$ROOT/etc/flint/config.toml" << 'EOF'
[global]
# Uncomment and set to your login username to run desktop services
# (hyprland, waybar, pipewire, dunst, etc.) as that user.
# session_user = "your-username"
EOF
    else
        cat > "$ROOT/etc/flint/config.toml" << EOF
[global]
# Desktop/session services (hyprland, waybar, pipewire, etc.) run as this user.
session_user = "$SESSION_USER"
EOF
        echo "[flint-install]   $ROOT/etc/flint/config.toml (session_user = $SESSION_USER) [ok]"
    fi
}

print_summary() {
    echo ""
    echo "=========================================="
    echo "  flint-init installed"
    echo "=========================================="
    echo ""
    echo "Installed to: $ROOT"
    echo "  $ROOT/usr/sbin/flint-init"
    echo "  $ROOT/usr/bin/flint-ctl"
    echo "  $ROOT/etc/flint/services/"
    echo "  $ROOT/etc/flint/config.toml  (session_user = ${SESSION_USER:-<not set>})"
    echo ""
    echo "To TEST (one boot, non-destructive):"
    echo "  At your bootloader, add to kernel parameters:"
    echo "    init=/usr/sbin/flint-init"
    echo "  Boot normally if anything goes wrong — just remove the parameter."
    echo ""
    echo "To make PERMANENT:"
    echo "  GRUB: set the 'Linux, with flint-init' entry as default"
    echo "  Other: make the init= parameter part of your default boot entry"
    echo ""
    echo "To REVERT:"
    echo "  Remove 'init=/usr/sbin/flint-init' from kernel parameters"
    echo "  GRUB: sudo rm /etc/grub.d/99-flint && sudo grub-mkconfig -o /boot/grub/grub.cfg"
    echo ""
}

main() {
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --root)     ROOT="$2"; shift 2 ;;
            --build)    ACQUIRE_MODE=build; shift ;;
            --download) ACQUIRE_MODE=download; shift ;;
            *) echo "[flint-install] error: unknown option: $1" >&2; exit 1 ;;
        esac
    done

    check_root
    detect_distro
    acquire_binary
    install_files
    write_flint_config
    configure_bootloader
    print_summary
}

if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
    main "$@"
fi
