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

    # If version is specifically nightly, we use the nightly tag
    if [[ "${BOSSA_VERSION:-}" == "nightly" ]]; then
        echo "nightly"
        return 0
    fi

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
        return 1
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

# Get download URL from GitHub API
get_download_url() {
    local version="$1"
    local platform="$2"
    local url="https://api.github.com/repos/${REPO}/releases/tags/${version}"
    
    if [ "$version" = "latest" ]; then
        url="https://api.github.com/repos/${REPO}/releases/latest"
    fi

    if ! command -v curl &>/dev/null; then
        return 1
    fi

    # Fetch release data and parse for asset URL
    # We search for an asset name that contains the platform string
    # and extract its browser_download_url
    curl -fsSL "$url" | \
        grep -E '"name":|"browser_download_url":' | \
        sed -E 's/.*"name": "([^"]+)".*/NAME:\1/; s/.*"browser_download_url": "([^"]+)".*/URL:\1/' | \
        awk -v pat="$platform" '
            /^NAME:/ { name=substr($0, 6) }
            /^URL:/ { 
                url=substr($0, 5)
                if (name ~ pat) { print url; exit }
            }
        '
}

# Download asset with fallback
fetch_asset() {
    local version="$1"
    local platform="$2"
    local ext="$3"
    local output="$4"
    
    # 1. Try standard naming convention
    local asset_name="${BINARY_NAME}-${platform}.${ext}"
    local url="https://github.com/${REPO}/releases/download/${version}/${asset_name}"
    
    info "Downloading ${asset_name}..."
    if download "$url" "$output"; then
        echo "$asset_name"
        return 0
    fi
    
    # 2. If failed, try to discover URL via API
    warn "Standard download failed, attempting to discover asset via API..."
    
    # For API search, simplify platform string if needed or use as is
    # e.g. linux-amd64 matches bossa-linux-amd64.tar.gz
    local api_url
    api_url=$(get_download_url "$version" "$platform")
    
    if [ -n "$api_url" ]; then
        local discovered_name="${api_url##*/}"
        info "Found asset: ${discovered_name}"
        if download "$api_url" "$output"; then
            echo "$discovered_name"
            return 0
        fi
    fi

    return 1
}

main() {
    echo ""
    printf "%s\n" "${BOLD}bossa installer${NC}"
    echo ""

    # Detect platform
    local platform
    platform=$(detect_platform)
    info "Detected platform: $platform"

    # Get version
    VERSION="${1:-${BOSSA_VERSION:-}}"
    if [ -z "$VERSION" ]; then
        info "Fetching latest version..."
        VERSION=$(get_latest_version)
        if [ -z "$VERSION" ]; then
            error "Failed to determine latest version"
        fi
    fi
    info "Installing version: $VERSION"

    # Determine file extension
    local ext
    if [[ "$platform" == "windows"* ]]; then
        ext="zip"
    else
        ext="tar.gz"
    fi

    # Create temp directory
    local tmpdir
    tmpdir=$(mktemp -d)
    # shellcheck disable=SC2064
    trap "rm -rf '$tmpdir'" EXIT

    # Download asset
    local asset_name
    if ! asset_name=$(fetch_asset "$VERSION" "$platform" "$ext" "${tmpdir}/asset"); then
        error "Failed to download asset for version $VERSION on platform $platform"
    fi

    # Determine checksum name
    local checksum_name="${asset_name}.sha256"
    local base_url="https://github.com/${REPO}/releases/download/${VERSION}"
    
    # If we discovered a URL via API, we might need to find the checksum URL similarly
    # But usually checksums follow standard naming. Let's try standard first.
    local checksum_url="${base_url}/${checksum_name}"

    # Download and verify checksum
    info "Downloading checksum..."
    download "$checksum_url" "${tmpdir}/${checksum_name}" 2>/dev/null || true
    verify_checksum "${tmpdir}/asset" "${tmpdir}/${checksum_name}"

    # Extract
    info "Extracting..."
    cd "$tmpdir"
    if [[ "$ext" == "zip" ]]; then
        unzip -q asset
    else
        tar -xzf asset
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
             # shellcheck disable=SC2016
            echo '  export PATH="$HOME/.local/bin:$PATH"'
            echo ""
            echo "  # zsh (~/.zshrc)"
             # shellcheck disable=SC2016
            echo '  export PATH="$HOME/.local/bin:$PATH"'
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
