#!/usr/bin/env bash
set -euo pipefail

REPO="kyxiaxiang/redcode"
BINARY="nocode"
INSTALL_DIR="${NOCODE_INSTALL_DIR:-$HOME/.local/bin}"

main() {
    echo "nocode installer"
    echo "---"

    detect_platform
    ensure_install_dir
    build_or_download
    verify_install

    echo "---"
    echo "installed: $INSTALL_DIR/$BINARY"
    echo "run: nocode --help"
}

detect_platform() {
    OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
    ARCH="$(uname -m)"
    case "$ARCH" in
        x86_64|amd64) ARCH="x86_64" ;;
        aarch64|arm64) ARCH="aarch64" ;;
        *) echo "unsupported architecture: $ARCH"; exit 1 ;;
    esac
    echo "platform: $OS/$ARCH"
}

ensure_install_dir() {
    mkdir -p "$INSTALL_DIR"
    if ! echo "$PATH" | tr ':' '\n' | grep -qx "$INSTALL_DIR"; then
        echo "warning: $INSTALL_DIR is not in PATH"
        echo "add to your shell profile: export PATH=\"$INSTALL_DIR:\$PATH\""
    fi
}

build_or_download() {
    if command -v cargo &>/dev/null; then
        echo "building from source with cargo..."
        cd "$(dirname "$0")/rust" 2>/dev/null || {
            echo "rust/ directory not found — cloning..."
            TMPDIR="$(mktemp -d)"
            git clone --depth 1 "https://github.com/$REPO.git" "$TMPDIR/redcode"
            cd "$TMPDIR/redcode/rust"
        }
        cargo build --release -p nocode
        cp "target/release/$BINARY" "$INSTALL_DIR/$BINARY"
        chmod +x "$INSTALL_DIR/$BINARY"
    else
        echo "cargo not found — install Rust first: https://rustup.rs"
        exit 1
    fi
}

verify_install() {
    if "$INSTALL_DIR/$BINARY" --status &>/dev/null; then
        echo "verification: ok"
    else
        echo "verification: binary runs but --status returned non-zero (may be expected)"
    fi
}

main "$@"
