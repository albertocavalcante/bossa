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

# Serve mdBook docs locally
docs-serve:
    mdbook serve docs --hostname 127.0.0.1 --port 3000

# Open local mdBook docs (assumes docs-serve is running)
docs-open:
    open http://127.0.0.1:3000

# Serve and open mdBook docs in one command
docs-preview:
    mdbook serve docs --hostname 127.0.0.1 --port 3000 &
    sleep 1
    open http://127.0.0.1:3000
    wait
