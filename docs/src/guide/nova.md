# Machine configuration (Nova)

Nova applies the selected sections of `config.toml` or `config.json`. Start with a narrow preview:

```bash
bossa nova --list-stages
bossa nova --only=defaults,symlinks --dry-run
bossa nova --only=defaults,symlinks
```

## Selected stages

| Stage       | Configuration or prerequisite                                                            |
| ----------- | ---------------------------------------------------------------------------------------- |
| `defaults`  | Scalar macOS defaults from `[defaults]`; arrays are rejected                             |
| `homebrew`  | Check for an existing Homebrew installation                                              |
| `packages`  | Taps, essential packages, formulas, casks and fonts from `[packages.brew]`               |
| `cellar`    | Configured synchronization is blocked pending preservation and recovery support          |
| `dotfiles`  | Configured synchronization is blocked pending a safe acquisition and dependency executor |
| `symlinks`  | Native dotfile links from `[symlinks]`                                                   |
| `dock`      | Apps and folders from `[dock]`                                                           |
| `handlers`  | File handlers from `[handlers]`                                                          |
| `ecosystem` | pnpm globals, GitHub CLI extensions and VS Code extensions                               |

Aliases are `brew` and `essential` for `packages`, `stow` for `symlinks`, and `pnpm` for
`ecosystem`. The package stage reads the unified configuration; use `bossa brew apply` for a
Brewfile. Other stages displayed as planned by `--list-stages` cannot be selected yet.

`--only` and `--skip` take comma-separated names and cannot be combined. Unknown, empty and
unsupported names return an error. Selecting an alias selects the whole corresponding stage.
Without either option, Nova selects all supported stages.

## Prerequisites and previews

Nova checks the Homebrew, cellar and dotfiles prerequisites before applying resources. Missing Homebrew blocks its
stage and configured Homebrew packages. Install it explicitly and retry; Nova does not fetch or
execute an installer. Creating a `bossa brew` client also never installs Homebrew implicitly.

Configured cellar and dotfiles synchronization currently block the selected Nova scope. An
unconfigured section needs no synchronization. To preview other resources while these stages
are blocked, select them explicitly:

```bash
bossa nova --only=defaults,dock --dry-run
```

`--dry-run` does not install Homebrew or synchronize cellar/dotfiles. Adding `--yes` still makes
no changes. A blocked preview returns a nonzero result with the unmet prerequisite; it does not
claim the machine is configured. Read-only previews never rewrite configuration.

## Applying resources

Remove `--dry-run` to request changes. `--yes` skips confirmation for that request. `--quiet`
controls output and does not grant consent. `-j` sets the requested resource job count.

Execution observation, verification, scheduling and completion reporting are being hardened in
the next modernization slice. Successful preview alone is not evidence of completed setup or a
persisted finish receipt. Preserve the failed command and its diagnostics when investigating an
incomplete apply; do not assume it rolled back all effects.
