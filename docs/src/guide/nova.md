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
| `dock`      | Explicit preferences, ordered apps and folder options from `[dock]`                      |
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
controls output and does not grant consent. Closed or unavailable interactive input blocks apply
unless `--yes` was explicitly selected. `-j` is a requested upper bound; resource effects are
currently serialized until their independence is demonstrated.

The resource executor retains observation errors, verifies postconditions after effects, and
reports failed, blocked or cancelled work as incomplete. Restarts require selected parents with
verified changes; filtered or failed parents do not trigger them. An accepted restart signal
request does not verify service health.

Reports separate execution, possible effects and verification. A command can fail after making
changes; preserve those effects and reobserve before retrying. The report is not a persisted
finish receipt and does not authorize automatic cleanup.

The native runner now limits captured output to 8 MiB and command lifetime to 30 minutes by
default. Its Linux/macOS captured jobs are cancelled on interrupt/termination, with no rollback
claim for prior effects. Inherited terminal jobs keep foreground behavior and supervise only
the direct child. Kernel cleanup can delay termination; escaped sessions and credential changes
are outside this ownership contract.

Selected Nova adapters use this runner. Each observation command has a 15-second deadline and an 8 MiB
combined output budget. Nonzero exits and invalid inventories block the selected scope before
effects. Configured restart/elevation features require `killall`/`sudo` executables before any
selected effects, even on an unchanged preview. This check does not invoke them or acquire
privileges. Defaults and Dock use typed XML preferences with byte, depth, event and duplicate-key
checks. Missing keys are absent only after a successful snapshot; a failed domain export stays
unavailable, including on systems where exporting a missing domain fails.

Unprivileged global tool observations and effects use the same captured HOME, locale and package-manager
selection policy. Corepack project selection and auto-pinning, Homebrew automatic updates and
known update notifiers are disabled in child environments. Observations also disable Corepack
network acquisition. These controls are not a sandbox for installed third-party tools.

Homebrew reads receipt files and validated opt links locally, including aliases and tap identities.
It uses only shell-fast installation-path queries during observation. An absent package requires
existing local formula/cask metadata to resolve its identity; explicitly install or update its
tap before retrying when that metadata is unavailable. Preview never downloads API metadata or
evaluates formula definitions. Casks also require a unique latest timestamped installed record;
Ruby records remain bounded opaque data. Recorded installation verification does not prove
executable, application or Ruby definition health. Package intent
must be a canonical lowercase Homebrew token or `owner/tap/token` (without `homebrew-` repository prefixes), a pnpm package name, a GitHub `owner/repository`,
or a VS Code `publisher.extension`. Unsupported URLs, local package paths and ecosystem version
pins are rejected before installation. Native handler configuration uses UTIs and bundle IDs.

Dock app order is one verified layout action; unselected tiles are retained. Folder view,
display and sort options are verified. Ambiguous dockutil selectors are blocked. Preferences
such as `autohide`, `tilesize`, `minimize_effect` and `show_recents` are managed only when
explicitly configured. Configured symlink packages must exist; discovery is bounded to 64
directory levels and 100,000 entries. Conflicting files or links are preserved for explicit
reconciliation.

Native cask elevation and elevated user-domain defaults are blocked before effects, including
when selected by a sudo allowlist. Running Homebrew as root is unsupported, and elevated user
defaults can target root's preference namespace. These need an explicit executor that preserves
the observed owner and target; custom `sudo.operations` are also unsupported.

The shared executor supports a scoped privilege adapter. Its native lifecycle retains foreground
job control, uses noninteractive execution after acquisition, bounds output, and attempts timestamp
invalidation after the batch, failed acquisition and handled termination. Failed invalidation makes
the invocation incomplete. Lifecycle fixtures use fake privilege commands in isolated child
environments; they do not qualify a native cask/system-preference executor. This does not prove
termination of children that change credentials or escape supervision, roll back effects, or
establish a durable recovery record.
