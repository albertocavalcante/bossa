# Bossa

**Unified CLI for managing your development environment.**

Bossa (Portuguese for "style" or "flair") brings together machine bootstrapping, package management, repository collections, and storage management into a single, declarative tool.

## Why Bossa?

Setting up and maintaining a development environment involves many moving parts:

- Installing and updating packages (Homebrew, npm, pip)
- Managing dotfiles and configurations
- Cloning and organizing reference repositories
- Handling storage across local SSD, iCloud, and external drives
- Bootstrapping new machines consistently

Bossa unifies these concerns with a declarative, idempotent approach.

## Key Features

| Feature         | Description                                          |
| --------------- | ---------------------------------------------------- |
| **Nova**        | Bootstrap a new machine in 16 stages                 |
| **Brew**        | Declarative Homebrew management with drift detection |
| **Collections** | Manage groups of git repositories                    |
| **Storage**     | Unified view of SSD, iCloud, and external drives     |
| **iCloud**      | Download, evict, and manage iCloud Drive files       |
| **Manifest**    | Hash files and find duplicates across locations      |
| **Doctor**      | Health checks for all systems                        |

## Quick Example

```bash
# Bootstrap a new machine
bossa nova

# Check system status
bossa status

# Apply desired state
bossa apply

# Manage Homebrew packages
bossa brew apply
bossa brew audit

# Sync repository collections
bossa collections sync refs
```

## Design Principles

1. **Declarative**: Define desired state, let bossa figure out how to get there
2. **Idempotent**: Run commands multiple times safely
3. **Parallel**: Operations run concurrently where possible
4. **Observable**: Clear status reporting and drift detection

## Getting Started

- [Installation](getting-started/installation.md) - Install bossa on your system
- [Quick Start](getting-started/quickstart.md) - Get up and running in minutes

## License

MIT
