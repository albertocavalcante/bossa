# Default: show available recipes
default:
    @just --list

# Development build
build:
    cargo build --locked

# Release build (optimized)
release:
    cargo build --release --locked

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
    cargo test --workspace --locked

# Lint every workspace target
lint:
    cargo clippy --workspace --all-targets --locked -- -D warnings

# Check formatting across the workspace
fmt-check:
    cargo fmt --all -- --check

# Check for issues
check: fmt-check lint

# Build API documentation with warnings treated as errors
doc-check:
    RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked

# Verify the declared minimum supported Rust version
msrv:
    cargo +1.90.0 check --workspace --all-targets --locked
    cargo +1.90.0 check --manifest-path tests/fixtures/sdk-consumer/Cargo.toml --all-targets --locked
    cargo +1.90.0 check --manifest-path tests/fixtures/sdk-consumer/Cargo.toml --all-targets --features native --locked

# Verify tagged action pins and upstream history for documented untagged pins
workflow-check:
    sh tools/scripts/check-actions.sh

# Test action verification failures without GitHub access
workflow-test:
    bash tools/scripts/check-actions-test.sh

# Format code
fmt:
    cargo fmt --all

# Build with Bazel
bazel-build:
    bazel build //:bossa --config=ci

# Test with Bazel
bazel-test:
    bazel test //...

# Qualify the lean SDK and path-based consumer preparation.
sdk-consumer-check:
    cargo test -p bossa-sdk --no-default-features --lib --locked
    cargo clippy -p bossa-sdk --no-default-features --lib --locked -- -D warnings
    cargo fmt --manifest-path tests/fixtures/sdk-consumer/Cargo.toml --all -- --check
    cargo run --manifest-path tests/fixtures/sdk-consumer/Cargo.toml --locked
    cargo test --manifest-path tests/fixtures/sdk-consumer/Cargo.toml --features native --locked
    bash -o pipefail -c 'output=$(cargo tree --manifest-path tests/fixtures/sdk-consumer/Cargo.toml -p bossa-sdk --locked -e normal --prefix none --format "{p}") || exit; if printf "%s\n" "$output" | grep -E "^(bossa-process|declarative|dirs|rustix|blake3|tempfile|crossterm|rusqlite|reqwest|ureq|zip|tar|brewkit) v"; then echo "Unexpected lean SDK dependency" >&2; exit 1; fi'
    bash -o pipefail -c 'output=$(cargo tree --manifest-path tests/fixtures/sdk-consumer/Cargo.toml --no-default-features --locked -e normal --prefix none --format "{p}") || exit; if printf "%s\n" "$output" | grep -E "^(bossa-process|dirs|rustix|blake3|tempfile|crossterm|rusqlite|reqwest|ureq|zip|tar|brewkit) v"; then echo "Unexpected lean consumer dependency" >&2; exit 1; fi'

# Run all checks (CI-style)
ci: check test doc-check sdk-consumer-check

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
