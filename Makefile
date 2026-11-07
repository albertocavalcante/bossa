.PHONY: all build release install clean test check ci

# Default target
all: release

# Development build
build:
	just build

# Release build (optimized)
release:
	just release

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
	just test

# Check for issues
check:
	just check

# Run the canonical workspace checks
ci:
	just ci
