.PHONY: build release install clean

# Default target
all: release

# Development build
build:
	cargo build

# Release build (optimized)
release:
	cargo build --release

# Install to ~/bin (assumes it's in PATH)
install: release
	@mkdir -p $(HOME)/bin
	@cp target/release/bossa $(HOME)/bin/bossa
	@chmod +x $(HOME)/bin/bossa
	@echo "✓ Installed bossa to ~/bin/bossa"
	@echo "  Run 'bossa --help' to get started"

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
