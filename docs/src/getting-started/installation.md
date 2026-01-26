# Installation

## Homebrew (Recommended)

The easiest way to install bossa on macOS:

```bash
brew install albertocavalcante/tap/bossa
```

> **Note:** Use the full tap path to avoid conflict with homebrew-core's `bossa` (a flash programmer tool).

## Pre-built Binaries

Download pre-built binaries from the [GitHub Releases](https://github.com/albertocavalcante/bossa/releases) page.

### macOS

```bash
# Apple Silicon (arm64)
curl -LO https://github.com/albertocavalcante/bossa/releases/latest/download/bossa-darwin-arm64.tar.gz
tar xzf bossa-darwin-arm64.tar.gz
sudo mv bossa /usr/local/bin/

# Intel (amd64)
curl -LO https://github.com/albertocavalcante/bossa/releases/latest/download/bossa-darwin-amd64.tar.gz
tar xzf bossa-darwin-amd64.tar.gz
sudo mv bossa /usr/local/bin/
```

### Linux

```bash
# x86_64
curl -LO https://github.com/albertocavalcante/bossa/releases/latest/download/bossa-linux-amd64.tar.gz
tar xzf bossa-linux-amd64.tar.gz
sudo mv bossa /usr/local/bin/

# ARM64
curl -LO https://github.com/albertocavalcante/bossa/releases/latest/download/bossa-linux-aarch64.tar.gz
tar xzf bossa-linux-aarch64.tar.gz
sudo mv bossa /usr/local/bin/
```

## From Source

### Using Cargo

```bash
# Clone the repository
git clone https://github.com/albertocavalcante/bossa.git
cd bossa

# Install with cargo
cargo install --path .
```

### Using Bazel

```bash
# Clone the repository
git clone https://github.com/albertocavalcante/bossa.git
cd bossa

# Build and install
bazel run //:install
```

## Shell Completions

Generate shell completions for your preferred shell:

```bash
# Bash
bossa completions bash >> ~/.bashrc

# Zsh
bossa completions zsh >> ~/.zshrc

# Fish
bossa completions fish > ~/.config/fish/completions/bossa.fish

# PowerShell
bossa completions powershell >> $PROFILE
```

## Verify Installation

```bash
bossa --version
bossa --help
```

## Updating

### Homebrew

```bash
brew upgrade bossa
```

### Cargo

```bash
cargo install --path . --force
```
