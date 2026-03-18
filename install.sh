#!/bin/sh
# superharness installer
# Usage:
#   curl -sSf https://raw.githubusercontent.com/backmeupplz/superharness/main/install.sh | sh
#   sh install.sh

set -e

REPO="backmeupplz/superharness"
BIN_NAME="superharness"
INSTALL_DIR="${HOME}/.local/bin"

# ── helpers ────────────────────────────────────────────────────────────────────

say() { printf '\033[1;32m==> \033[0m%s\n' "$*"; }
warn() { printf '\033[1;33mwarn:\033[0m %s\n' "$*" >&2; }
err() { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 1; }

need() {
    command -v "$1" >/dev/null 2>&1 || err "'$1' is required but not found. Please install it and retry."
}

# ── detect OS & arch ──────────────────────────────────────────────────────────

detect_target() {
    local os arch

    case "$(uname -s)" in
        Linux)  os="linux" ;;
        Darwin) os="macos" ;;
        *)      err "Unsupported OS: $(uname -s)" ;;
    esac

    case "$(uname -m)" in
        x86_64)          arch="x86_64" ;;
        aarch64|arm64)   arch="aarch64" ;;
        *)               err "Unsupported architecture: $(uname -m)" ;;
    esac

    echo "${os}-${arch}"
}

# Map friendly name to the release asset name used in GitHub Releases
asset_name() {
    local target="$1"
    case "$target" in
        linux-x86_64)   echo "superharness-x86_64-unknown-linux-gnu" ;;
        linux-aarch64)  echo "superharness-aarch64-unknown-linux-gnu" ;;
        macos-x86_64)   echo "superharness-x86_64-apple-darwin" ;;
        macos-aarch64)  echo "superharness-aarch64-apple-darwin" ;;
        *)              err "Unknown target: $target" ;;
    esac
}

# ── fetch latest version tag from GitHub API ──────────────────────────────────

latest_version() {
    if command -v curl >/dev/null 2>&1; then
        curl -sSf "https://api.github.com/repos/${REPO}/releases/latest" \
            | grep '"tag_name"' \
            | head -1 \
            | sed 's/.*"tag_name": *"\([^"]*\)".*/\1/'
    elif command -v wget >/dev/null 2>&1; then
        wget -qO- "https://api.github.com/repos/${REPO}/releases/latest" \
            | grep '"tag_name"' \
            | head -1 \
            | sed 's/.*"tag_name": *"\([^"]*\)".*/\1/'
    else
        err "curl or wget is required for downloading. Please install one and retry."
    fi
}

# ── download helper ───────────────────────────────────────────────────────────

download() {
    local url="$1" dest="$2"
    if command -v curl >/dev/null 2>&1; then
        curl -fSL --progress-bar -o "$dest" "$url"
    elif command -v wget >/dev/null 2>&1; then
        wget -q --show-progress -O "$dest" "$url"
    else
        err "curl or wget is required. Please install one and retry."
    fi
}

# ── fallback: cargo install ───────────────────────────────────────────────────

install_via_cargo() {
    say "Falling back to 'cargo install ${BIN_NAME}'..."
    need cargo
    cargo install "${BIN_NAME}" --locked
    say "Installed via cargo. Binary is in \$(cargo root)/bin — make sure that is on your PATH."
    exit 0
}

# ── PATH setup hint ───────────────────────────────────────────────────────────

ensure_path_hint() {
    case ":${PATH}:" in
        *":${INSTALL_DIR}:"*)
            return 0
            ;;
    esac

    warn "${INSTALL_DIR} is not in your PATH."
    printf '\nAdd it by running ONE of the following (and then restart your shell):\n\n'
    printf '  # bash\n'
    printf '  echo '"'"'export PATH="$HOME/.local/bin:$PATH"'"'"' >> ~/.bashrc\n\n'
    printf '  # zsh\n'
    printf '  echo '"'"'export PATH="$HOME/.local/bin:$PATH"'"'"' >> ~/.zshrc\n\n'
    printf '  # fish\n'
    printf '  fish_add_path ~/.local/bin\n\n'
}

# ── main ──────────────────────────────────────────────────────────────────────

main() {
    say "Detecting platform..."
    local target
    target="$(detect_target)"
    say "Detected: ${target}"

    say "Fetching latest release version..."
    local tag
    tag="$(latest_version)" || install_via_cargo
    [ -n "$tag" ] || install_via_cargo
    say "Latest version: ${tag}"

    local asset
    asset="$(asset_name "$target")"
    local url="https://github.com/${REPO}/releases/download/${tag}/${asset}"

    say "Downloading ${asset}..."
    local tmpfile
    tmpfile="$(mktemp /tmp/superharness-XXXXXX)"
    download "$url" "$tmpfile" || {
        rm -f "$tmpfile"
        warn "Pre-built binary not available for ${target}."
        install_via_cargo
    }

    say "Installing to ${INSTALL_DIR}/${BIN_NAME}..."
    mkdir -p "${INSTALL_DIR}"
    mv "$tmpfile" "${INSTALL_DIR}/${BIN_NAME}"
    chmod 755 "${INSTALL_DIR}/${BIN_NAME}"

    say "Installed ${BIN_NAME} ${tag} to ${INSTALL_DIR}/${BIN_NAME}"
    ensure_path_hint

    printf '\nRun: %s --help\n' "${BIN_NAME}"
}

main "$@"
