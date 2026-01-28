# Default: show available recipes
default:
    @just --list

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

# ─────────────────────────────────────────────────────────────────────────────
# Editor & Browser shortcuts
# ─────────────────────────────────────────────────────────────────────────────

# Open repo on GitHub
open:
    open "https://github.com/albertocavalcante/bossa"

# Open in VS Code
vscode:
    code .

# Open in Zed
zed:
    zed .

# Open in Cursor
cursor:
    cursor .

# ─────────────────────────────────────────────────────────────────────────────
# GitHub shortcuts
# ─────────────────────────────────────────────────────────────────────────────

# Open GitHub issues
issues:
    open "https://github.com/albertocavalcante/bossa/issues"

# Open GitHub pull requests
prs:
    open "https://github.com/albertocavalcante/bossa/pulls"

# Open GitHub Actions
actions:
    open "https://github.com/albertocavalcante/bossa/actions"

# Open GitHub releases
releases:
    open "https://github.com/albertocavalcante/bossa/releases"

# Trigger nightly build workflow
nightly:
    gh workflow run nightly.yml

# Create a new issue (opens in browser)
new-issue:
    open "https://github.com/albertocavalcante/bossa/issues/new"

# Compare current branch for PR
pr-create:
    #!/usr/bin/env bash
    branch=$(git branch --show-current)
    open "https://github.com/albertocavalcante/bossa/compare/${branch}?expand=1"

# ─────────────────────────────────────────────────────────────────────────────
# Development helpers
# ─────────────────────────────────────────────────────────────────────────────

# Run bossa with arguments
run *args:
    cargo run -- {{args}}

# Run bossa release build with arguments
run-release *args:
    cargo run --release -- {{args}}

# Watch for changes and run tests
watch:
    cargo watch -x test

# Watch for changes and run clippy
watch-check:
    cargo watch -x clippy

# Auto-fix clippy warnings and format code
fix:
    cargo clippy --fix --allow-dirty --allow-staged
    cargo fmt
    dprint fmt

# Update dependencies
update-deps:
    cargo update

# Check for outdated dependencies
outdated:
    cargo outdated -R

# ─────────────────────────────────────────────────────────────────────────────
# Git helpers
# ─────────────────────────────────────────────────────────────────────────────

# Show git status
status:
    git status -sb

# Show recent commits
log:
    git log --oneline -20

# Show diff
diff:
    git diff

# Amend last commit (keep message)
amend:
    git commit --amend --no-edit

# ─────────────────────────────────────────────────────────────────────────────
# Hooks
# ─────────────────────────────────────────────────────────────────────────────

# Install git hooks via lefthook
hooks-install:
    lefthook install

# Run pre-commit hooks manually
hooks-run:
    lefthook run pre-commit
