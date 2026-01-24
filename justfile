# Default target
default: release

# Development build
build:
    cargo build

# Release build (optimized)
release:
    cargo build --release

# Install to ~/bin (assumes it's in PATH)
install: release
    mkdir -p $HOME/bin
    cp target/release/bossa $HOME/bin/bossa
    chmod +x $HOME/bin/bossa
    @echo "Installed bossa to ~/bin/bossa"
    @echo "Run 'bossa --help' to get started"

# Install via cargo (to ~/.cargo/bin)
install-cargo:
    cargo install --path .

# Clean build artifacts
clean:
    cargo clean

# Run tests
test:
    cargo test

# Check for issues
check:
    cargo clippy
    cargo fmt --check

# Format code
fmt:
    cargo fmt

# Build with Bazel
bazel-build:
    bazel build //:bossa

# Test with Bazel
bazel-test:
    bazel test //...

# Run all checks (CI-style)
ci: check test
