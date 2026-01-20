#!/bin/bash
# Install locally-built bossa binary
# Usage: bazel run //:install
#
# Environment variables:
#   INSTALL_DIR - Installation directory (default: ~/.local/bin)

set -e

INSTALL_DIR="${INSTALL_DIR:-$HOME/.local/bin}"
BOSSA_PATH="${BUILD_WORKSPACE_DIRECTORY}/bazel-bin/bossa"

# Colors
if [ -t 1 ]; then
    GREEN='\033[0;32m'
    BLUE='\033[0;34m'
    YELLOW='\033[0;33m'
    NC='\033[0m'
else
    GREEN='' BLUE='' YELLOW='' NC=''
fi

info() { printf "${BLUE}==>${NC} %s\n" "$1"; }
success() { printf "${GREEN}==>${NC} %s\n" "$1"; }
warn() { printf "${YELLOW}warning:${NC} %s\n" "$1"; }

# Build if needed
if [ ! -f "$BOSSA_PATH" ]; then
    info "Building bossa..."
    (cd "$BUILD_WORKSPACE_DIRECTORY" && bazel build //:bossa)
fi

# Install
info "Installing to ${INSTALL_DIR}..."
mkdir -p "$INSTALL_DIR"

rm -f "$INSTALL_DIR/bossa"
cp "$BOSSA_PATH" "$INSTALL_DIR/bossa"
chmod +x "$INSTALL_DIR/bossa"

success "Installed bossa to ${INSTALL_DIR}/"

# Check PATH
case ":$PATH:" in
    *":$INSTALL_DIR:"*) ;;
    *)
        echo ""
        warn "$INSTALL_DIR is not in your PATH."
        echo "Add it: export PATH=\"\$PATH:$INSTALL_DIR\""
        ;;
esac

# Show version
echo ""
"$INSTALL_DIR/bossa" --version
