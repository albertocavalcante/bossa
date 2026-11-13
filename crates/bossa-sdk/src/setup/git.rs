//! Bounded machine protocols. No shell parsing or implicit fetch/repair.

use anyhow::{Context, Result, bail, ensure};
use std::collections::HashSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::{CommandRequest, IoMode, ProcessRunner};

const MAX_OUTPUT: usize = 8 * 1024 * 1024;

pub(super) fn is_local_locator(url: &str) -> bool {
    // Git's scp spelling requires the first colon to precede every slash.
    Path::new(url).is_absolute()
        || url
            .split_once(':')
            .is_none_or(|(prefix, _)| prefix.contains('/'))
}

pub(super) struct Git {
    pub observer: Arc<dyn ProcessRunner + Send + Sync>,
    pub mutator: Arc<dyn ProcessRunner + Send + Sync>,
    pub cwd: PathBuf,
}

#[derive(Debug)]
pub(super) struct Registration {
    pub path: PathBuf,
    pub branch: Option<String>,
    pub head: Option<String>,
    pub unavailable: bool,
}

fn field(bytes: &[u8]) -> Result<&str> {
    std::str::from_utf8(bytes).context("Git returned a non-UTF-8 metadata field")
}

fn path_field(bytes: &[u8]) -> Result<PathBuf> {
    ensure!(!bytes.is_empty() && !bytes.contains(&0), "Invalid Git path");
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        Ok(PathBuf::from(OsString::from_vec(bytes.to_vec())))
    }
    #[cfg(not(unix))]
    {
        Ok(PathBuf::from(field(bytes)?))
    }
}

fn line(bytes: &[u8]) -> Result<&[u8]> {
    bytes.strip_suffix(b"\n").context("Incomplete Git response")
}

impl Git {
    fn run(&self, path: Option<&Path>, args: Vec<OsString>, mutate: bool) -> Result<Vec<u8>> {
        let mut all: Vec<OsString> = [
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "gc.auto=0",
            "-c",
            "maintenance.auto=false",
            "-c",
            "submodule.recurse=false",
            "-c",
            "protocol.ext.allow=never",
        ]
        .into_iter()
        .map(Into::into)
        .collect();
        if !mutate {
            all.push("--no-lazy-fetch".into());
        }
        if let Some(path) = path {
            all.extend(["-C".into(), path.as_os_str().to_os_string()]);
        }
        all.extend(args);
        let runner = if mutate {
            &self.mutator
        } else {
            &self.observer
        };
        let output = runner
            .run(
                &CommandRequest {
                    program: "git".into(),
                    args: all,
                    working_dir: self.cwd.clone(),
                },
                IoMode::Capture,
            )
            .context("Git execution unavailable; any partial files are retained")?;
        ensure!(
            output.status.success(),
            "Git operation failed ({}); any partial files are retained",
            output.status
        );
        ensure!(
            output.stdout.len() <= MAX_OUTPUT
                && output.stderr.len() <= MAX_OUTPUT.saturating_sub(output.stdout.len()),
            "Git output exceeded its budget"
        );
        Ok(output.stdout)
    }

    fn read(&self, path: &Path, args: &[&str]) -> Result<Vec<u8>> {
        self.run(
            Some(path),
            args.iter().map(|s| (*s).into()).collect(),
            false,
        )
    }

    pub fn check_observation_capability(&self) -> Result<()> {
        self.run(None, vec!["--version".into()], false)
            .context("Git must support --no-lazy-fetch for read-only observations")?;
        Ok(())
    }

    pub fn path(&self, path: &Path, option: &str) -> Result<PathBuf> {
        let output = self.read(path, &["rev-parse", "--path-format=absolute", option])?;
        let result = path_field(line(&output)?)?;
        ensure!(
            result.is_absolute(),
            "Git returned a relative repository path"
        );
        result
            .canonicalize()
            .context("Git repository path is unavailable")
    }

    pub fn kind(&self, path: &Path, bare: bool) -> Result<()> {
        let output = self.read(path, &["rev-parse", "--is-bare-repository"])?;
        ensure!(
            output
                == if bare {
                    b"true\n".as_slice()
                } else {
                    b"false\n".as_slice()
                },
            "Unexpected repository kind at {}",
            path.display()
        );
        Ok(())
    }

    pub fn branch(&self, path: &Path, branch: &str) -> Result<()> {
        let output = self.read(path, &["symbolic-ref", "-q", "HEAD"])?;
        ensure!(
            line(&output)? == format!("refs/heads/{branch}").as_bytes(),
            "Wrong checked-out branch at {}",
            path.display()
        );
        self.commit(path).map(|_| ())
    }

    pub fn commit(&self, path: &Path) -> Result<String> {
        let output = self.read(path, &["rev-parse", "--verify", "HEAD^{commit}"])?;
        let id = field(line(&output)?)?;
        ensure!(valid_oid(id), "Invalid Git commit identity");
        Ok(id.into())
    }

    pub fn require_branch(&self, path: &Path, branch: &str) -> Result<()> {
        let reference = format!("refs/heads/{branch}");
        let output = self.read(
            path,
            &[
                "for-each-ref",
                "--format=%(refname)%00%(objectname)%00",
                "--",
                &reference,
            ],
        )?;
        let fields: Vec<_> = output.split(|b| *b == 0).collect();
        ensure!(
            fields.len() == 3
                && fields[0] == reference.as_bytes()
                && valid_oid(field(fields[1])?)
                && fields[2] == b"\n",
            "Requested branch is missing or ambiguous: {branch}; no branch base is guessed"
        );
        Ok(())
    }

    pub fn origin(&self, path: &Path, expected: &str) -> Result<()> {
        let output = self.read(
            path,
            &["config", "--local", "--no-includes", "--null", "--list"],
        )?;
        ensure!(
            output.last() == Some(&0),
            "Incomplete local Git configuration"
        );
        let mut urls = Vec::new();
        for entry in output[..output.len() - 1].split(|b| *b == 0) {
            let (key, value) = entry.split_once_byte(b'\n');
            if key.eq_ignore_ascii_case(b"remote.origin.url") {
                urls.push(value.context("Origin has no value")?);
            }
        }
        ensure!(urls.len() == 1, "Missing or ambiguous local origin");
        let actual = field(urls[0])?;
        let actual = if is_local_locator(expected) {
            ensure!(
                Path::new(actual).is_absolute(),
                "Local origin must be absolute"
            );
            self.locator(actual)?
        } else {
            actual.into()
        };
        ensure!(
            actual == self.locator(expected)?,
            "Repository origin does not match configured identity at {}",
            path.display()
        );
        Ok(())
    }

    pub fn locator(&self, url: &str) -> Result<String> {
        if is_local_locator(url) {
            let path = super::resolve_location(&self.cwd.join(url))?;
            Ok(path
                .to_str()
                .context("A local repository locator requires a UTF-8 path")?
                .into())
        } else {
            Ok(url.into())
        }
    }

    pub fn registrations(&self, bare: &Path) -> Result<Vec<Registration>> {
        parse_registrations(&self.read(bare, &["worktree", "list", "--porcelain", "-z"])?)
    }

    pub fn clone_repo(&self, url: &str, destination: &Path, options: Vec<OsString>) -> Result<()> {
        let mut args = vec![
            "clone".into(),
            "--origin".into(),
            "origin".into(),
            "--no-local".into(),
            "--template=".into(),
        ];
        args.extend(options);
        args.extend([
            "--".into(),
            self.locator(url)?.into(),
            destination.as_os_str().to_os_string(),
        ]);
        self.run(None, args, true).map(|_| ())
    }

    pub fn add(&self, bare: &Path, path: &Path, branch: &str) -> Result<()> {
        self.run(
            Some(bare),
            vec![
                "worktree".into(),
                "add".into(),
                "--".into(),
                path.as_os_str().to_os_string(),
                branch.into(),
            ],
            true,
        )
        .map(|_| ())
    }
}

// Compatibility with the minimum Rust toolchain; values may contain newlines.
trait SplitByte {
    fn split_once_byte(&self, byte: u8) -> (&[u8], Option<&[u8]>);
}
impl SplitByte for [u8] {
    fn split_once_byte(&self, byte: u8) -> (&[u8], Option<&[u8]>) {
        self.iter()
            .position(|b| *b == byte)
            .map_or((self, None), |n| (&self[..n], Some(&self[n + 1..])))
    }
}

fn valid_oid(id: &str) -> bool {
    [40, 64].contains(&id.len()) && id.bytes().all(|b| b.is_ascii_hexdigit())
}

fn parse_registrations(output: &[u8]) -> Result<Vec<Registration>> {
    ensure!(
        output.len() <= MAX_OUTPUT && output.ends_with(b"\0\0"),
        "Incomplete or oversized worktree inventory"
    );
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    let mut pending: Option<Registration> = None;
    let mut keys = HashSet::new();
    for entry in output[..output.len() - 1].split(|b| *b == 0) {
        if entry.is_empty() {
            let record = pending.take().context("Empty worktree record")?;
            let bare = keys.contains(b"bare".as_slice());
            let detached = keys.contains(b"detached".as_slice());
            ensure!(
                if bare {
                    !detached && record.head.is_none() && record.branch.is_none()
                } else {
                    record.head.is_some() && (record.branch.is_some() != detached)
                },
                "Incomplete or contradictory worktree record"
            );
            if let Some(branch) = &record.branch {
                super::validate_branch(
                    branch
                        .strip_prefix("refs/heads/")
                        .context("Invalid registered branch reference")?,
                )?;
            }
            ensure!(
                seen.insert(record.path.clone()),
                "Duplicate worktree registration"
            );
            ensure!(result.len() < 10_000, "Too many worktrees");
            result.push(record);
            keys.clear();
            continue;
        }
        let (key, value) = entry.split_once_byte(b' ');
        ensure!(keys.insert(key.to_vec()), "Duplicate worktree field");
        if key == b"worktree" {
            ensure!(pending.is_none(), "Missing worktree record boundary");
            let path = path_field(value.context("Missing registered path")?)?;
            ensure!(path.is_absolute(), "Relative worktree registration");
            pending = Some(Registration {
                path,
                branch: None,
                head: None,
                unavailable: false,
            });
        } else {
            let record = pending.as_mut().context("Worktree field before path")?;
            match key {
                b"branch" => record.branch = Some(field(value.context("Missing branch")?)?.into()),
                b"HEAD" => {
                    let head = field(value.context("Missing HEAD")?)?;
                    ensure!(valid_oid(head), "Invalid registered HEAD");
                    record.head = Some(head.into());
                }
                b"locked" | b"prunable" => record.unavailable = true,
                b"bare" | b"detached" => ensure!(value.is_none(), "Invalid worktree marker"),
                _ => bail!("Unsupported worktree inventory field"),
            }
        }
    }
    ensure!(
        pending.is_none() && !result.is_empty(),
        "Incomplete worktree inventory"
    );
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inventory_is_nul_delimited_bounded_and_never_accepts_duplicate_or_partial_records() {
        let oid = "a".repeat(40);
        let valid = format!(
            "worktree /bare\0bare\0\0worktree /line\nbreak\0HEAD {oid}\0branch refs/heads/main\0\0"
        );
        let parsed = parse_registrations(valid.as_bytes()).unwrap();
        assert_eq!(parsed[1].path, Path::new("/line\nbreak"));
        for invalid in [
            "worktree /bare\0bare\0",
            "worktree /a\0bare\0bare\0\0",
            "worktree /a\0bare\0\0worktree /a\0bare\0\0",
            "worktree relative\0bare\0\0",
            "worktree /a\0surprise\0\0",
            "HEAD abc\0\0",
            "worktree /a\0\0",
            "worktree /a\0bare\0branch refs/heads/main\0\0",
        ] {
            assert!(
                parse_registrations(invalid.as_bytes()).is_err(),
                "{invalid:?}"
            );
        }
        use std::fmt::Write as _;
        let mut many = String::new();
        for n in 0..10_000 {
            write!(many, "worktree /{n}\0bare\0\0").unwrap();
        }
        assert_eq!(parse_registrations(many.as_bytes()).unwrap().len(), 10_000);
        assert!(
            parse_registrations(format!("{many}worktree /overflow\0bare\0\0").as_bytes()).is_err()
        );
    }
}
