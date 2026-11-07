//! Local Homebrew state. Path-only brew queries avoid Ruby metadata loading,
//! implicit API downloads, and inventories that silently omit broken installs.

use anyhow::{Context, Result, bail};
use bossa::ConfigSource;
use std::fs;
use std::path::{Path, PathBuf};

use super::brew_package::BrewPackageType;

pub(super) struct BrewPaths {
    pub prefix: PathBuf,
    pub cellar: PathBuf,
    pub caskroom: PathBuf,
    pub repository: PathBuf,
}

impl BrewPaths {
    fn discover() -> Result<Self> {
        fn query(flag: &str) -> Result<PathBuf> {
            let output = crate::runner::observe("brew", [flag])?;
            let text = std::str::from_utf8(&output).context("Invalid Homebrew path encoding")?;
            let text = text.strip_suffix('\n').unwrap_or(text);
            let path = PathBuf::from(text);
            if !path.is_absolute() || text.contains(['\n', '\r', '\0']) {
                bail!("Homebrew path query returned no unambiguous absolute path");
            }
            Ok(path)
        }
        Ok(Self {
            prefix: query("--prefix")?,
            cellar: query("--cellar")?,
            caskroom: query("--caskroom")?,
            repository: query("--repository")?,
        })
    }

    pub(super) fn native() -> Result<&'static Self> {
        // Only installation roots are captured. Installed state is always read
        // afresh for preconditions/postconditions, including after failures.
        static PATHS: std::sync::OnceLock<Result<BrewPaths, String>> = std::sync::OnceLock::new();
        PATHS
            .get_or_init(|| Self::discover().map_err(|error| format!("{error:#}")))
            .as_ref()
            .map_err(|error| anyhow::anyhow!("{error}"))
    }

    fn tap_path(&self, tap: &str) -> Result<PathBuf> {
        let (owner, name) = tap.split_once('/').context("Invalid Homebrew tap")?;
        Ok(self
            .repository
            .join("Library/Taps")
            .join(owner)
            .join(format!("homebrew-{name}")))
    }

    pub(super) fn installed(&self, kind: BrewPackageType, wanted: &str) -> Result<bool> {
        if kind == BrewPackageType::Tap {
            let path = self.tap_path(wanted)?;
            if !directory(&path)? {
                return Ok(false);
            }
            let git = path.join(".git");
            if optional_metadata(&git)?.is_none_or(|metadata| !metadata.is_dir()) {
                bail!(
                    "Homebrew tap needs a supported Git checkout with a real .git directory: {}",
                    path.display()
                );
            }
            let output = crate::runner::observe(
                "git",
                [
                    std::ffi::OsString::from("-C"),
                    path.as_os_str().into(),
                    "rev-parse".into(),
                    "--show-toplevel".into(),
                    "--absolute-git-dir".into(),
                ],
            )?;
            let text = std::str::from_utf8(&output)
                .context("Invalid Homebrew tap Git identity encoding")?;
            let lines: Vec<_> = text.lines().collect();
            if lines.len() != 2
                || Path::new(lines[0]).canonicalize()? != path.canonicalize()?
                || Path::new(lines[1]).canonicalize()? != git.canonicalize()?
            {
                bail!("Homebrew tap Git identity does not match its checkout");
            }
            return Ok(true);
        }
        let pieces: Vec<_> = wanted.split('/').collect();
        let token = *pieces.last().context("Missing Homebrew token")?;
        let requested_tap = (pieces.len() == 3).then(|| format!("{}/{}", pieces[0], pieces[1]));
        if self.installed_token(kind, token, requested_tap.as_deref())? {
            return Ok(true);
        }
        if kind == BrewPackageType::Formula {
            let opt = self.prefix.join("opt").join(token);
            if let Some(metadata) = optional_metadata(&opt)? {
                if !metadata.file_type().is_symlink() {
                    bail!("Homebrew opt record is not a symlink: {}", opt.display());
                }
                let target = opt
                    .canonicalize()
                    .context("Homebrew opt record is unavailable")?;
                let cellar = self
                    .cellar
                    .canonicalize()
                    .context("Homebrew Cellar is unavailable")?;
                let relative = target
                    .strip_prefix(&cellar)
                    .context("Homebrew opt record escapes the Cellar")?;
                let components: Vec<_> = relative.components().collect();
                if components.len() != 2 {
                    bail!("Homebrew opt record does not identify one installed keg");
                }
                let canonical = components[0]
                    .as_os_str()
                    .to_str()
                    .context("Invalid Homebrew keg identity")?;
                if !directory(&target)? {
                    bail!("Homebrew opt record does not identify a directory");
                }
                check_receipt(
                    &target.join("INSTALL_RECEIPT.json"),
                    requested_tap.as_deref(),
                    false,
                )?;
                if !self.installed_token(kind, canonical, requested_tap.as_deref())? {
                    bail!("Homebrew alias has no complete installation");
                }
                return Ok(true);
            }
        }
        // An absent token could be an alias. Require existing local tap metadata
        // to resolve identity before proposing installation; never fetch it here.
        let tap = requested_tap
            .as_deref()
            .unwrap_or(if kind == BrewPackageType::Cask {
                "homebrew/cask"
            } else {
                "homebrew/core"
            });
        let root = self.tap_path(tap)?;
        let section = if kind == BrewPackageType::Cask {
            "Casks"
        } else {
            "Formula"
        };
        let first = token
            .chars()
            .next()
            .context("Empty Homebrew token")?
            .to_string();
        let candidates = [
            root.join(section).join(format!("{token}.rb")),
            root.join(section).join(first).join(format!("{token}.rb")),
            root.join("Aliases").join(token),
        ];
        let mut identity = None;
        for candidate in candidates {
            if optional_metadata(&candidate)?.is_none() {
                continue;
            }
            let resolved = candidate
                .canonicalize()
                .context("Local Homebrew identity metadata is unavailable")?;
            let canonical_root = root
                .canonicalize()
                .context("Local Homebrew tap is unavailable")?;
            if resolved
                .extension()
                .is_none_or(|extension| extension != "rb")
                || !resolved.starts_with(canonical_root.join(section))
                || !fs::metadata(&resolved)?.is_file()
            {
                bail!("Homebrew identity metadata escapes its formula/cask directory");
            }
            let name = resolved
                .file_stem()
                .and_then(std::ffi::OsStr::to_str)
                .context("Invalid local Homebrew identity")?
                .to_string();
            if identity.replace(name).is_some() {
                bail!("Ambiguous local Homebrew identity metadata");
            }
        }
        let canonical = identity.context("Local Homebrew identity metadata is unavailable; install/update the corresponding tap explicitly before previewing an absent package")?;
        self.installed_token(kind, &canonical, Some(tap))
    }

    fn installed_token(
        &self,
        kind: BrewPackageType,
        token: &str,
        tap: Option<&str>,
    ) -> Result<bool> {
        if token != token.to_ascii_lowercase()
            || !token.starts_with(|value: char| value.is_ascii_alphanumeric())
            || token
                .chars()
                .any(|value| !value.is_ascii_alphanumeric() && !"@+._-".contains(value))
        {
            bail!("Unsupported local Homebrew token identity");
        }
        let root = if kind == BrewPackageType::Cask {
            &self.caskroom
        } else {
            &self.cellar
        };
        let path = root.join(token);
        if !directory(&path)? {
            return Ok(false);
        }
        if kind == BrewPackageType::Cask {
            let receipt = check_receipt(&path.join(".metadata/INSTALL_RECEIPT.json"), tap, true)?;
            check_cask_record(&path, token, &receipt)?;
            return Ok(true);
        }
        let mut count = 0;
        let mut bytes = 0;
        let mut versions = 0;
        let mut observed_tap = None;
        for entry in
            fs::read_dir(&path).context("Could not enumerate installed Homebrew versions")?
        {
            count += 1;
            if count > 1000 {
                bail!("Homebrew version inventory exceeds entry limit");
            }
            let entry = entry?;
            if entry.file_name() == ".DS_Store" {
                continue;
            }
            if !entry.file_type()?.is_dir() {
                bail!("Unexpected object in installed Homebrew versions");
            }
            let receipt = check_receipt(&entry.path().join("INSTALL_RECEIPT.json"), tap, false)?;
            if observed_tap
                .as_ref()
                .is_some_and(|previous: &String| previous != &receipt.tap)
            {
                bail!("Ambiguous tap identities in installed Homebrew versions");
            }
            observed_tap = Some(receipt.tap);
            bytes += receipt.bytes;
            versions += 1;
            if bytes > 16 * 1024 * 1024 {
                bail!("Homebrew receipts exceed aggregate byte limit");
            }
        }
        if versions == 0 {
            bail!("Homebrew package directory has no installed version receipts");
        }
        Ok(true)
    }
}

fn optional_metadata(path: &Path) -> Result<Option<fs::Metadata>> {
    match fs::symlink_metadata(path) {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            bossa::confirm_missing_parent(path)?;
            Ok(None)
        }
        Err(error) => {
            Err(error).with_context(|| format!("Homebrew state unavailable: {}", path.display()))
        }
    }
}

fn directory(path: &Path) -> Result<bool> {
    match optional_metadata(path)? {
        None => Ok(false),
        Some(metadata) if metadata.is_dir() => Ok(true),
        Some(_) => bail!(
            "Expected a Homebrew directory; preserved unexpected object at {}",
            path.display()
        ),
    }
}

struct Receipt {
    bytes: usize,
    tap: String,
    version: Option<String>,
}

fn check_receipt(path: &Path, expected_tap: Option<&str>, cask: bool) -> Result<Receipt> {
    if optional_metadata(path)?.is_none_or(|metadata| !metadata.is_file()) {
        bail!(
            "Homebrew installation receipt must be a regular file: {}",
            path.display()
        );
    }
    let text = bossa::FileConfigSource::default()
        .read(path)?
        .with_context(|| {
            format!(
                "Homebrew installation receipt is missing: {}",
                path.display()
            )
        })?;
    let value: serde_json::Value = bossa::decode_unique_json(text.as_bytes())
        .context("Invalid Homebrew installation receipt")?;
    let source = value
        .get("source")
        .and_then(serde_json::Value::as_object)
        .context("Homebrew receipt has no source identity")?;
    let tap = source
        .get("tap")
        .and_then(serde_json::Value::as_str)
        .filter(|tap| !tap.is_empty())
        .context("Homebrew receipt has no tap identity")?;
    let pieces: Vec<_> = tap.split('/').collect();
    if pieces.len() != 2
        || pieces.iter().any(|piece| {
            !piece.starts_with(|value: char| value.is_ascii_alphanumeric())
                || piece
                    .chars()
                    .any(|value| !value.is_ascii_alphanumeric() && !"._-".contains(value))
        })
    {
        bail!("Invalid Homebrew receipt tap identity");
    }
    if expected_tap.is_some_and(|expected| !expected.eq_ignore_ascii_case(tap)) {
        bail!(
            "Installed Homebrew package has a different tap identity; preserved for explicit reconciliation"
        );
    }
    if cask
        && source
            .get("version")
            .and_then(serde_json::Value::as_str)
            .is_none_or(str::is_empty)
    {
        bail!("Homebrew cask receipt has no installed version");
    }
    Ok(Receipt {
        bytes: text.len(),
        tap: tap.to_string(),
        version: source
            .get("version")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
    })
}

fn bounded_entries(path: &Path, count: &mut usize) -> Result<Vec<fs::DirEntry>> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(path)? {
        *count += 1;
        if *count > 1000 {
            bail!("Homebrew cask metadata exceeds entry limit");
        }
        entries.push(entry?);
    }
    Ok(entries)
}

fn check_cask_record(path: &Path, token: &str, receipt: &Receipt) -> Result<()> {
    let metadata = path.join(".metadata");
    let mut count = 0;
    let mut latest: Option<(String, String, PathBuf)> = None;
    let mut ambiguous = false;
    for version in bounded_entries(&metadata, &mut count)? {
        let kind = version.file_type()?;
        if kind.is_file()
            && [
                "INSTALL_RECEIPT.json",
                "config.json",
                "LATEST_DOWNLOAD_SHA256",
                ".DS_Store",
            ]
            .iter()
            .any(|name| version.file_name() == *name)
        {
            continue;
        }
        if !kind.is_dir() {
            bail!("Unsupported object in Homebrew cask metadata");
        }
        let name = version
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("Invalid cask version encoding"))?;
        for timestamp in bounded_entries(&version.path(), &mut count)? {
            let stamp = timestamp
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("Invalid cask timestamp encoding"))?;
            if stamp == ".DS_Store" && timestamp.file_type()?.is_file() {
                continue;
            }
            let bytes = stamp.as_bytes();
            if !timestamp.file_type()?.is_dir()
                || bytes.len() != 18
                || bytes[14] != b'.'
                || !bytes[..14]
                    .iter()
                    .chain(&bytes[15..])
                    .all(u8::is_ascii_digit)
            {
                bail!("Unsupported Homebrew cask metadata timestamp");
            }
            match &latest {
                Some((previous, _, _)) if previous == &stamp => ambiguous = true,
                Some((previous, _, _)) if previous > &stamp => {}
                _ => {
                    latest = Some((stamp, name.clone(), timestamp.path()));
                    ambiguous = false;
                }
            }
        }
    }
    let (_, version, latest) =
        latest.context("Homebrew cask receipt has no installed cask record")?;
    if ambiguous {
        bail!("Ambiguous latest Homebrew cask timestamp");
    }
    if receipt.version.as_deref() != Some(version.as_str()) {
        bail!("Homebrew cask receipt and installed record disagree on version");
    }
    let records = latest.join("Casks");
    if !directory(&records)? {
        bail!("Homebrew installed cask record directory is missing");
    }
    let mut record = None;
    for extension in ["json", "internal.json", "rb"] {
        let file = records.join(format!("{token}.{extension}"));
        if let Some(metadata) = optional_metadata(&file)? {
            if !metadata.is_file() {
                bail!("Homebrew installed cask record must be a regular file");
            }
            if record.replace((file, extension)).is_some() {
                bail!("Ambiguous installed cask records");
            }
        }
    }
    let (file, extension) = record.context("Homebrew installed cask record is missing")?;
    let text = bossa::FileConfigSource::default()
        .read(&file)?
        .context("Homebrew installed cask record disappeared")?;
    if text.trim().is_empty() {
        bail!("Homebrew installed cask record is empty");
    }
    if extension != "rb" {
        let value: serde_json::Value =
            bossa::decode_unique_json(text.as_bytes()).context("Invalid installed cask JSON")?;
        let object = value
            .as_object()
            .context("Installed cask JSON must be an object")?;
        if let Some(value) = object.get("version").filter(|value| !value.is_null())
            && value.as_str() != Some(version.as_str())
        {
            bail!("Installed cask JSON version disagrees with its path");
        }
        for (field, expected) in [("token", token), ("tap", receipt.tap.as_str())] {
            if object
                .get(field)
                .is_some_and(|value| value.as_str() != Some(expected))
            {
                bail!("Installed cask JSON identity disagrees with its recorded installation");
            }
        }
        if object
            .get("artifacts")
            .is_some_and(|value| !value.is_null() && !value.is_array())
            || object
                .get("url_specs")
                .is_some_and(|value| !value.is_null() && !value.is_object())
        {
            bail!("Invalid installed cask artifact metadata");
        }
    }
    // Ruby snapshots remain bounded opaque records, matching Homebrew's presence
    // rule. No Ruby is evaluated and application/definition health is unproven.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, BrewPaths) {
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path();
        let paths = BrewPaths {
            prefix: root.into(),
            cellar: root.join("Cellar"),
            caskroom: root.join("Caskroom"),
            repository: root.join("repo"),
        };
        for path in [
            &paths.cellar,
            &paths.caskroom,
            &paths.repository,
            &paths.prefix.join("opt"),
        ] {
            fs::create_dir_all(path).unwrap();
        }
        (temp, paths)
    }

    fn receipt(path: &Path, source: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, source).unwrap();
    }

    #[test]
    fn selected_receipts_are_fresh_typed_and_tap_bound() {
        let (_temp, paths) = fixture();
        let kind = BrewPackageType::Formula;
        assert!(paths.installed(kind, "node").is_err()); // unknown alias/canonical identity
        let metadata = paths
            .repository
            .join("Library/Taps/homebrew/homebrew-core/Formula/n/node.rb");
        receipt(&metadata, "metadata only");
        assert!(!paths.installed(kind, "node").unwrap());
        let tab = paths.cellar.join("node/1/INSTALL_RECEIPT.json");
        receipt(&tab, r#"{"source":{"tap":"homebrew/core"}}"#);
        assert!(paths.installed(kind, "homebrew/core/node").unwrap());
        assert!(paths.installed(kind, "other/tap/node").is_err());
        fs::write(
            &tab,
            r#"{"source":{"tap":"homebrew/core","tap":"other/tap"}}"#,
        )
        .unwrap();
        assert!(paths.installed(kind, "node").is_err());
        fs::remove_file(&tab).unwrap();
        assert!(paths.installed(kind, "node").is_err());
    }

    #[test]
    #[cfg(unix)]
    fn alias_links_must_resolve_to_receipt_bearing_kegs_inside_cellar() {
        let (temp, paths) = fixture();
        let kind = BrewPackageType::Formula;
        let keg = paths.cellar.join("node/1");
        receipt(
            &keg.join("INSTALL_RECEIPT.json"),
            r#"{"source":{"tap":"homebrew/core"}}"#,
        );
        let opt = paths.prefix.join("opt/nodejs");
        std::os::unix::fs::symlink(&keg, &opt).unwrap();
        assert!(paths.installed(kind, "nodejs").unwrap());
        assert!(paths.installed(kind, "homebrew/core/nodejs").unwrap());
        assert!(paths.installed(kind, "other/tap/nodejs").is_err());
        fs::remove_file(&opt).unwrap();
        std::os::unix::fs::symlink(temp.path().join("outside"), &opt).unwrap();
        assert!(paths.installed(kind, "nodejs").is_err());
        fs::create_dir(temp.path().join("outside")).unwrap();
        assert!(paths.installed(kind, "nodejs").is_err());
    }

    #[test]
    fn cask_receipts_need_observed_version_and_package_directories_need_receipts() {
        let (_temp, paths) = fixture();
        let tab = paths.caskroom.join("app/.metadata/INSTALL_RECEIPT.json");
        receipt(&tab, r#"{"source":{"tap":"homebrew/cask","version":"1"}}"#);
        assert!(paths.installed(BrewPackageType::Cask, "app").is_err());
        let record = paths
            .caskroom
            .join("app/.metadata/1/20261107000000.000/Casks/app.json");
        receipt(&record, r#"{"version":"1","artifacts":[]}"#);
        fs::write(
            tab.parent().unwrap().join("LATEST_DOWNLOAD_SHA256"),
            "opaque cache hint",
        )
        .unwrap();
        assert!(paths.installed(BrewPackageType::Cask, "app").unwrap());
        fs::write(&record, r#"{"version":"2"}"#).unwrap();
        assert!(paths.installed(BrewPackageType::Cask, "app").is_err());
        fs::remove_file(&record).unwrap();
        receipt(&record.with_extension("internal.json"), "{}");
        assert!(paths.installed(BrewPackageType::Cask, "app").unwrap());
        fs::remove_file(record.with_extension("internal.json")).unwrap();
        receipt(
            &record.with_extension("rb"),
            "opaque Ruby recorded installation; never evaluated",
        );
        assert!(paths.installed(BrewPackageType::Cask, "app").unwrap());
        fs::write(&tab, r#"{"source":{"tap":"homebrew/cask"}}"#).unwrap();
        assert!(paths.installed(BrewPackageType::Cask, "app").is_err());
        fs::create_dir(paths.cellar.join("empty")).unwrap();
        fs::write(paths.cellar.join("empty/.DS_Store"), "").unwrap();
        assert!(paths.installed(BrewPackageType::Formula, "empty").is_err());
    }

    #[test]
    fn latest_cask_record_must_be_unique_complete_and_match_the_receipt() {
        let (_temp, paths) = fixture();
        let base = paths.caskroom.join("app/.metadata");
        let tab = base.join("INSTALL_RECEIPT.json");
        receipt(&tab, r#"{"source":{"tap":"homebrew/cask","version":"1"}}"#);
        receipt(&base.join("1/20261107000000.000/Casks/app.json"), "{}");
        assert!(paths.installed(BrewPackageType::Cask, "app").unwrap());
        let newest = base.join("2/20261107000001.000/Casks/app.json");
        receipt(&newest, "{}");
        assert!(paths.installed(BrewPackageType::Cask, "app").is_err());
        fs::write(&tab, r#"{"source":{"tap":"homebrew/cask","version":"2"}}"#).unwrap();
        assert!(paths.installed(BrewPackageType::Cask, "app").unwrap());
        fs::remove_file(&newest).unwrap();
        assert!(paths.installed(BrewPackageType::Cask, "app").is_err());
        receipt(&newest, "{}");
        receipt(&base.join("3/20261107000001.000/Casks/app.json"), "{}");
        assert!(paths.installed(BrewPackageType::Cask, "app").is_err());
    }

    #[test]
    fn a_tap_directory_with_an_invalid_git_file_is_unavailable() {
        let (_temp, paths) = fixture();
        let tap = paths.tap_path("owner/repo").unwrap();
        fs::create_dir_all(&tap).unwrap();
        fs::write(tap.join(".git"), "").unwrap();
        assert!(paths.installed(BrewPackageType::Tap, "owner/repo").is_err());
    }
}
