#!/usr/bin/env bash
# nocode installer — auto-detect platform/arch, download, verify, install.
# Usage: curl -fsSL https://raw.githubusercontent.com/telagod/nocode/main/install.sh | bash
# Options: --version <ver>  Install specific version (default: latest)
#          --dir <path>     Install directory (default: ~/.nocode/bin)

set -euo pipefail

REPO="telagod/nocode"
INSTALL_DIR="${HOME}/.nocode/bin"
VERSION=""

# Parse arguments
while [[ $# -gt 0 ]]; do
    case "$1" in
        --version) VERSION="$2"; shift 2 ;;
        --dir) INSTALL_DIR="$2"; shift 2 ;;
        --help) echo "Usage: install.sh [--version <ver>] [--dir <path>]"; exit 0 ;;
        *) echo "Unknown option: $1"; exit 1 ;;
    esac
done

# Detect platform
detect_platform() {
    local os arch
    os="$(uname -s)"
    arch="$(uname -m)"

    case "$os" in
        Linux)  os="linux" ;;
        Darwin) os="darwin" ;;
        MINGW*|MSYS*|CYGWIN*) os="win32" ;;
        *) echo "Unsupported OS: $os"; exit 1 ;;
    esac

    case "$arch" in
        x86_64|amd64)  arch="x64" ;;
        aarch64|arm64) arch="arm64" ;;
        *) echo "Unsupported architecture: $arch"; exit 1 ;;
    esac

    echo "${os}-${arch}"
}

# Get latest version from GitHub API
get_latest_version() {
    local url="https://api.github.com/repos/${REPO}/releases/latest"
    curl -fsSL "$url" | grep '"tag_name"' | head -1 | sed 's/.*"tag_name": *"//;s/".*//'
}

# Download and install
main() {
    local platform
    platform="$(detect_platform)"
    echo "Detected platform: ${platform}"

    if [[ -z "$VERSION" ]]; then
        echo "Fetching latest version..."
        VERSION="$(get_latest_version)"
        if [[ -z "$VERSION" ]]; then
            echo "Failed to determine latest version"
            exit 1
        fi
    fi
    echo "Installing nocode ${VERSION}..."

    local binary_name="nocode"
    if [[ "$platform" == win32-* ]]; then
        binary_name="nocode.exe"
    fi

    local download_url="https://github.com/${REPO}/releases/download/${VERSION}/nocode-${platform}"
    if [[ "$platform" == win32-* ]]; then
        download_url="${download_url}.exe"
    fi

    # Create install directory
    mkdir -p "$INSTALL_DIR"

    local tmp_file
    tmp_file="$(mktemp)"
    trap 'rm -f "$tmp_file" "${tmp_file}.sha256"' EXIT

    echo "Downloading ${download_url}..."
    if ! curl -fsSL -o "$tmp_file" "$download_url"; then
        echo "Download failed. Check that version ${VERSION} exists."
        exit 1
    fi

    # Verify checksum if available
    local checksum_url="${download_url}.sha256"
    if curl -fsSL -o "${tmp_file}.sha256" "$checksum_url" 2>/dev/null; then
        echo "Verifying checksum..."
        local expected actual
        expected="$(cat "${tmp_file}.sha256" | awk '{print $1}')"
        if command -v sha256sum &>/dev/null; then
            actual="$(sha256sum "$tmp_file" | awk '{print $1}')"
        elif command -v shasum &>/dev/null; then
            actual="$(shasum -a 256 "$tmp_file" | awk '{print $1}')"
        else
            echo "Warning: no sha256 tool found, skipping checksum verification"
            actual="$expected"
        fi
        if [[ "$expected" != "$actual" ]]; then
            echo "Checksum mismatch!"
            echo "  Expected: ${expected}"
            echo "  Actual:   ${actual}"
            exit 1
        fi
        echo "Checksum verified."
    else
        echo "No checksum file available, skipping verification."
    fi

    # Install binary
    local dest="${INSTALL_DIR}/${binary_name}"
    mv "$tmp_file" "$dest"
    chmod +x "$dest"

    echo ""
    echo "Installed nocode ${VERSION} to ${dest}"
    echo ""

    # Check if install dir is in PATH
    if ! echo "$PATH" | tr ':' '\n' | grep -qx "$INSTALL_DIR"; then
        echo "Add to your PATH:"
        echo ""
        echo "  export PATH=\"${INSTALL_DIR}:\$PATH\""
        echo ""
        echo "Or add to your shell profile (~/.bashrc, ~/.zshrc, etc.)"
    else
        echo "Run: nocode --tui"
    fi
}

main
