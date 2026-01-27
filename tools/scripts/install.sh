#!/bin/bash
# Install bossa from GitHub releases
# Usage: curl -fsSL https://raw.githubusercontent.com/albertocavalcante/bossa/main/tools/scripts/install.sh | bash
#
# Environment variables:
#   BOSSA_VERSION  - Version to install (default: latest)
#   BOSSA_DIR      - Installation directory (default: ~/.local/bin)
#   BOSSA_REPO     - GitHub repository (default: albertocavalcante/bossa)

set -euo pipefail

REPO="${BOSSA_REPO:-albertocavalcante/bossa}"
INSTALL_DIR="${BOSSA_DIR:-$HOME/.local/bin}"
VERSION="${BOSSA_VERSION:-}"
BINARY_NAME="bossa"

# Colors (disabled if not a terminal)
if [ -t 1 ]; then
    RED='\033[0;31m'
    GREEN='\033[0;32m'
    BLUE='\033[0;34m'
    YELLOW='\033[0;33m'
    BOLD='\033[1m'
    NC='\033[0m'
else
    RED='' GREEN='' BLUE='' YELLOW='' BOLD='' NC=''
fi

info()    { printf "${BLUE}==>${NC} %s\n" "$1"; }
success() { printf "${GREEN}==>${NC} %s\n" "$1"; }
warn()    { printf "${YELLOW}warning:${NC} %s\n" "$1"; }
error()   { printf "${RED}error:${NC} %s\n" "$1" >&2; exit 1; }

# Detect OS and architecture
detect_platform() {
    local os arch

    case "$(uname -s)" in
        Linux*)  os="linux" ;;
        Darwin*) os="darwin" ;;
        MINGW*|MSYS*|CYGWIN*) os="windows" ;;
        *) error "Unsupported OS: $(uname -s)" ;;
    esac

    case "$(uname -m)" in
        x86_64|amd64)  arch="amd64" ;;
        aarch64|arm64) arch="arm64" ;;
        *) error "Unsupported architecture: $(uname -m)" ;;
    esac

    # Map to release asset names
    case "${os}-${arch}" in
        linux-amd64)  echo "linux-amd64" ;;
        linux-arm64)  echo "linux-aarch64" ;;
        darwin-amd64) echo "darwin-amd64" ;;
        darwin-arm64) echo "darwin-arm64" ;;
        windows-amd64) echo "windows-amd64" ;;
        *) error "Unsupported platform: ${os}-${arch}" ;;
    esac
}

# Get latest version from GitHub API
get_latest_version() {
    local url="https://api.github.com/repos/${REPO}/releases/latest"

    if command -v curl &>/dev/null; then
        curl -fsSL "$url" | grep '"tag_name"' | sed -E 's/.*"([^"]+)".*/\1/'
    elif command -v wget &>/dev/null; then
        wget -qO- "$url" | grep '"tag_name"' | sed -E 's/.*"([^"]+)".*/\1/'
    else
        error "Neither curl nor wget found. Please install one of them."
    fi
}

# Download file
download() {
    local url="$1"
    local output="$2"

    if command -v curl &>/dev/null; then
        curl -fsSL --progress-bar "$url" -o "$output"
    elif command -v wget &>/dev/null; then
        wget -q --show-progress "$url" -O "$output"
    else
        error "Neither curl nor wget found."
    fi
}

# Verify checksum
verify_checksum() {
    local file="$1"
    local checksum_file="$2"

    if [ ! -f "$checksum_file" ]; then
        warn "Checksum file not found, skipping verification"
        return 0
    fi

    local expected actual
    expected=$(cat "$checksum_file" | awk '{print $1}')

    if command -v sha256sum &>/dev/null; then
        actual=$(sha256sum "$file" | awk '{print $1}')
    elif command -v shasum &>/dev/null; then
        actual=$(shasum -a 256 "$file" | awk '{print $1}')
    else
        warn "Neither sha256sum nor shasum found, skipping verification"
        return 0
    fi

    if [ "$expected" != "$actual" ]; then
        error "Checksum mismatch!\nExpected: $expected\nActual:   $actual"
    fi

    success "Checksum verified"
}

main() {
    echo ""
    printf "${BOLD}bossa installer${NC}\n"
    echo ""

    # Detect platform
    local platform
    platform=$(detect_platform)
    info "Detected platform: $platform"

    # Get version
    if [ -z "$VERSION" ]; then
        info "Fetching latest version..."
        VERSION=$(get_latest_version)
        if [ -z "$VERSION" ]; then
            error "Failed to determine latest version"
        fi
    fi
    info "Installing version: $VERSION"

    # Determine file extension and asset name
    local ext asset_name checksum_name
    if [[ "$platform" == "windows"* ]]; then
        ext="zip"
    else
        ext="tar.gz"
    fi
    asset_name="${BINARY_NAME}-${platform}.${ext}"
    checksum_name="${asset_name}.sha256"

    # Build download URLs
    local base_url="https://github.com/${REPO}/releases/download/${VERSION}"
    local asset_url="${base_url}/${asset_name}"
    local checksum_url="${base_url}/${checksum_name}"

    # Create temp directory
    local tmpdir
    tmpdir=$(mktemp -d)
    trap "rm -rf '$tmpdir'" EXIT

    # Download asset
    info "Downloading ${asset_name}..."
    download "$asset_url" "${tmpdir}/${asset_name}"

    # Download and verify checksum
    info "Downloading checksum..."
    download "$checksum_url" "${tmpdir}/${checksum_name}" 2>/dev/null || true
    verify_checksum "${tmpdir}/${asset_name}" "${tmpdir}/${checksum_name}"

    # Extract
    info "Extracting..."
    cd "$tmpdir"
    if [[ "$ext" == "zip" ]]; then
        unzip -q "${asset_name}"
    else
        tar -xzf "${asset_name}"
    fi

    # Install
    info "Installing to ${INSTALL_DIR}..."
    mkdir -p "$INSTALL_DIR"

    local binary_ext=""
    [[ "$platform" == "windows"* ]] && binary_ext=".exe"

    mv "${BINARY_NAME}${binary_ext}" "${INSTALL_DIR}/${BINARY_NAME}${binary_ext}"
    chmod +x "${INSTALL_DIR}/${BINARY_NAME}${binary_ext}"

    success "Installed bossa to ${INSTALL_DIR}/${BINARY_NAME}${binary_ext}"

    # Check PATH
    case ":$PATH:" in
        *":$INSTALL_DIR:"*) ;;
        *)
            echo ""
            warn "$INSTALL_DIR is not in your PATH"
            echo ""
            echo "Add it to your shell configuration:"
            echo ""
            echo "  # bash (~/.bashrc)"
            echo "  export PATH=\"\$HOME/.local/bin:\$PATH\""
            echo ""
            echo "  # zsh (~/.zshrc)"
            echo "  export PATH=\"\$HOME/.local/bin:\$PATH\""
            echo ""
            echo "  # fish (~/.config/fish/config.fish)"
            echo "  fish_add_path ~/.local/bin"
            echo ""
            ;;
    esac

    # Show version
    echo ""
    if command -v "${INSTALL_DIR}/${BINARY_NAME}" &>/dev/null || [ -x "${INSTALL_DIR}/${BINARY_NAME}" ]; then
        "${INSTALL_DIR}/${BINARY_NAME}" --version
    fi

    echo ""
    success "Installation complete!"
    echo ""
    echo "Get started:"
    echo "  bossa --help"
    echo "  bossa status"
    echo ""
}

main "$@"
