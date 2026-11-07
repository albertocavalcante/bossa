//! Dock resources observed through bounded, typed native preferences.

use anyhow::{Context, Result, bail};
use plist::{Dictionary, Value};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use super::{ApplyContext, ApplyResult, Resource, ResourceState};

struct Tile<'a> {
    position: usize,
    data: &'a Dictionary,
}

fn tile_path(data: &Dictionary) -> Result<Option<PathBuf>> {
    let Some(file) = data.get("file-data") else {
        return Ok(None); // URL and spacer tiles have no filesystem identity.
    };
    let file = file.as_dictionary().context("Invalid Dock file data")?;
    let value = file
        .get("_CFURLString")
        .and_then(Value::as_string)
        .context("Dock file has no path observation")?;
    let kind = file
        .get("_CFURLStringType")
        .and_then(Value::as_unsigned_integer)
        .context("Dock file has no URL type")?;
    match kind {
        0 => {
            let path = PathBuf::from(value);
            if !path.is_absolute() {
                bail!("Dock snapshot contains a relative file path");
            }
            Ok(Some(path))
        }
        15 => Ok(Some(
            url::Url::parse(value)
                .context("Invalid Dock file URL")?
                .to_file_path()
                .map_err(|()| anyhow::anyhow!("Dock URL is not a local file"))?,
        )),
        _ => bail!("Unsupported Dock path encoding"),
    }
}

fn find_tile<'a>(snapshot: &'a Dictionary, section: &str, path: &Path) -> Result<Option<Tile<'a>>> {
    let entries = snapshot
        .get(section)
        .and_then(Value::as_array)
        .context("Dock snapshot has no complete section inventory")?;
    if entries.len() > 10_000 {
        bail!("Dock inventory exceeds entry limit");
    }
    let mut found = None;
    for (index, entry) in entries.iter().enumerate() {
        let data = entry
            .as_dictionary()
            .and_then(|entry| entry.get("tile-data"))
            .and_then(Value::as_dictionary)
            .context("Invalid Dock tile observation")?;
        let observed = tile_path(data)?;
        let matches = match observed {
            Some(observed) if observed == path => true,
            Some(observed) => match observed.canonicalize() {
                Ok(actual) => actual == path,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
                Err(error) => return Err(error).context("Dock tile path is unavailable"),
            },
            None => false,
        };
        if matches {
            if found.is_some() {
                bail!("Duplicate Dock path; reconcile it explicitly before applying");
            }
            found = Some(Tile {
                position: index + 1,
                data,
            });
        }
    }
    Ok(found)
}

/// Mirror dockutil's label/bundle/raw-URL/decoded-path selectors, including
/// recent apps. Refuse ambiguous selection rather than update a different tile.
fn selector_count(snapshot: &Dictionary, selector: &str) -> Result<usize> {
    // dockutil uses Swift String equality, which compares canonical Unicode
    // equivalents. Byte equality can otherwise select an unintended sibling.
    let normalizer = icu_normalizer::ComposingNormalizer::new_nfc();
    let selector = normalizer.normalize(selector);
    let matches = |value: &str| normalizer.normalize(value) == selector;
    let mut count = 0;
    for section in ["persistent-apps", "recent-apps", "persistent-others"] {
        let Some(entries) = snapshot.get(section) else {
            if section == "recent-apps" {
                continue;
            }
            bail!("Dock snapshot has no complete selector inventory");
        };
        let entries = entries
            .as_array()
            .context("Invalid Dock selector inventory")?;
        if entries.len() > 10_000 {
            bail!("Dock inventory exceeds entry limit");
        }
        for entry in entries {
            let data = entry
                .as_dictionary()
                .and_then(|entry| entry.get("tile-data"))
                .and_then(Value::as_dictionary)
                .context("Invalid Dock selector observation")?;
            let label = data
                .get("file-label")
                .or_else(|| data.get("label"))
                .and_then(Value::as_string);
            let bundle = data.get("bundle-identifier").and_then(Value::as_string);
            let raw_url = data
                .get("file-data")
                .and_then(Value::as_dictionary)
                .and_then(|file| file.get("_CFURLString"))
                .or_else(|| {
                    data.get("url")
                        .and_then(Value::as_dictionary)
                        .and_then(|file| file.get("_CFURLString"))
                })
                .and_then(Value::as_string);
            let matches_url = if let Some(raw) = raw_url {
                let decoded = percent_encoding::percent_decode_str(raw)
                    .decode_utf8()
                    .context("Invalid Dock URL encoding")?;
                let decoded_path = url::Url::parse(raw)
                    .ok()
                    .map(|url| {
                        percent_encoding::percent_decode_str(url.path())
                            .decode_utf8()
                            .map(std::borrow::Cow::into_owned)
                    })
                    .transpose()
                    .context("Invalid Dock selector path encoding")?;
                matches(raw) || matches(&decoded) || decoded_path.as_deref().is_some_and(matches)
            } else {
                false
            };
            if label.is_some_and(matches) || bundle.is_some_and(matches) || matches_url {
                count += 1;
            }
        }
    }
    Ok(count)
}

fn unique_label(snapshot: &Dictionary, tile: &Tile<'_>) -> Result<String> {
    let label = tile
        .data
        .get("file-label")
        .and_then(Value::as_string)
        .context("Dock tile has no update label")?;
    super::native::identifier(label, "Dock label")?;
    if selector_count(snapshot, label)? != 1 {
        bail!("Ambiguous Dock selector; reconcile it explicitly before applying");
    }
    Ok(label.into())
}

fn desired_path(path: &str) -> Result<PathBuf> {
    let expanded = crate::paths::expand(path);
    if !expanded.is_absolute() {
        bail!("Dock paths must be absolute after expansion");
    }
    let metadata = std::fs::metadata(&expanded).context("Dock source is unavailable")?;
    if !metadata.is_dir() {
        bail!("Dock app/folder source must be a directory");
    }
    expanded
        .canonicalize()
        .context("Dock source identity is unavailable")
}

fn execute(args: Vec<OsString>) -> Result<()> {
    let output = crate::runner::capture("dockutil", args)?;
    if !output.status.success() {
        bail!("dockutil failed: {}; effects may be partial", output.status);
    }
    Ok(())
}

/// One ordered prefix is one action: moving a tile changes sibling positions.
#[derive(Debug, Clone)]
pub struct DockApps {
    pub paths: Vec<String>,
}

impl DockApps {
    fn expanded(&self) -> Result<Vec<PathBuf>> {
        if self.paths.len() > 1000 {
            bail!("Dock layout exceeds app limit");
        }
        let paths: Vec<_> = self
            .paths
            .iter()
            .map(|path| desired_path(path))
            .collect::<Result<_>>()?;
        if paths
            .iter()
            .any(|path| path.extension().is_none_or(|extension| extension != "app"))
        {
            bail!("Dock apps require application bundle directories ending in .app");
        }
        let mut unique = std::collections::HashSet::new();
        let mut labels = std::collections::HashSet::new();
        for path in &paths {
            let label = app_label(path)?;
            if !unique.insert(path)
                || !labels.insert(
                    icu_normalizer::ComposingNormalizer::new_nfc()
                        .normalize(&label)
                        .into_owned(),
                )
            {
                bail!("Duplicate Dock app path/label; reconcile the selected layout");
            }
        }
        Ok(paths)
    }

    fn is_satisfied(snapshot: &Dictionary, paths: &[PathBuf]) -> Result<bool> {
        let mut satisfied = true;
        for (index, path) in paths.iter().enumerate() {
            match find_tile(snapshot, "persistent-apps", path)? {
                Some(tile) => {
                    if tile.position != index + 1 {
                        unique_label(snapshot, &tile)?;
                        satisfied = false;
                    }
                }
                None => {
                    ensure_unused_label(snapshot, &app_label(path)?)?;
                    satisfied = false;
                }
            }
        }
        Ok(satisfied)
    }
}

fn app_label(path: &Path) -> Result<String> {
    let label = path
        .file_stem()
        .and_then(std::ffi::OsStr::to_str)
        .context("Dock app label must have a UTF-8 filename")?;
    super::native::identifier(label, "Dock app label")?;
    Ok(label.into())
}

fn ensure_unused_label(snapshot: &Dictionary, label: &str) -> Result<()> {
    if selector_count(snapshot, label)? != 0 {
        bail!(
            "Existing Dock selector belongs to a different path; preserved for explicit reconciliation"
        );
    }
    Ok(())
}

impl Resource for DockApps {
    fn id(&self) -> String {
        "dock:apps".into()
    }
    fn description(&self) -> String {
        format!("Place {} configured apps in Dock order", self.paths.len())
    }
    fn resource_type(&self) -> &'static str {
        "dock_apps"
    }
    fn current_state(&self) -> Result<ResourceState> {
        let paths = self.expanded()?;
        let snapshot = super::native::preferences("com.apple.dock")?;
        if Self::is_satisfied(&snapshot, &paths)? {
            Ok(self.desired_state())
        } else {
            crate::runner::require_command_at("dockutil", crate::paths::environment().home_dir()?)?;
            Ok(ResourceState::Modified {
                from: format!("{:?}", snapshot.get("persistent-apps")),
                to: format!("{:?}", self.paths),
            })
        }
    }
    fn desired_state(&self) -> ResourceState {
        ResourceState::Present {
            details: Some(format!("{:?}", self.paths)),
        }
    }
    fn apply(&self, ctx: &mut ApplyContext) -> Result<ApplyResult> {
        if ctx.dry_run {
            return Ok(ApplyResult::Skipped {
                reason: "Dry run".into(),
            });
        }
        let paths = self.expanded()?;
        let mut changed = false;
        for (index, path) in paths.iter().enumerate() {
            let snapshot = super::native::preferences("com.apple.dock")?;
            let tile = find_tile(&snapshot, "persistent-apps", path)?;
            let mut args: Vec<OsString> = if let Some(tile) = tile {
                if tile.position == index + 1 {
                    continue;
                }
                vec!["--move".into(), unique_label(&snapshot, &tile)?.into()]
            } else {
                ensure_unused_label(&snapshot, &app_label(path)?)?;
                vec![
                    "--add".into(),
                    path.as_os_str().into(),
                    "--section".into(),
                    "apps".into(),
                    "--type".into(),
                    "file".into(),
                ]
            };
            args.extend([
                "--no-restart".into(),
                "--position".into(),
                (index + 1).to_string().into(),
            ]);
            execute(args)?;
            changed = true;
        }
        Ok(if changed {
            ApplyResult::Modified
        } else {
            ApplyResult::NoChange
        })
    }
    fn can_parallelize(&self) -> bool {
        false
    }
}

#[derive(Debug, Clone)]
pub struct DockFolder {
    pub path: String,
    pub view: String,
    pub display: String,
    pub sort: String,
}

impl DockFolder {
    pub fn new(path: &str) -> Self {
        Self {
            path: path.into(),
            view: "grid".into(),
            display: "stack".into(),
            sort: "dateadded".into(),
        }
    }

    fn options(&self) -> Result<[(&'static str, u64); 3]> {
        // Numeric mappings are dockutil's FolderView/FolderDisplay/FolderSort.
        let view = match self.view.as_str() {
            "auto" => 0,
            "fan" => 1,
            "grid" => 2,
            "list" => 3,
            _ => bail!("Unsupported Dock folder view"),
        };
        let display = match self.display.as_str() {
            "stack" => 0,
            "folder" => 1,
            _ => bail!("Unsupported Dock folder display"),
        };
        let sort = match self.sort.as_str() {
            "name" => 1,
            "dateadded" => 2,
            "datemodified" => 3,
            "datecreated" => 4,
            "kind" => 5,
            _ => bail!("Unsupported Dock folder sort"),
        };
        Ok([
            ("showas", view),
            ("displayas", display),
            ("arrangement", sort),
        ])
    }
    fn matches(&self, tile: &Tile<'_>) -> Result<bool> {
        let mut matches = true;
        for (key, desired) in self.options()? {
            let current = tile
                .data
                .get(key)
                .and_then(Value::as_unsigned_integer)
                .context("Dock folder option is unavailable")?;
            matches &= current == desired;
        }
        Ok(matches)
    }
}

impl Resource for DockFolder {
    fn id(&self) -> String {
        format!("dock:folder:{}", self.path)
    }
    fn description(&self) -> String {
        format!("Configure Dock folder {}", self.path)
    }
    fn resource_type(&self) -> &'static str {
        "dock_folder"
    }
    fn current_state(&self) -> Result<ResourceState> {
        self.options()?;
        let path = desired_path(&self.path)?;
        let snapshot = super::native::preferences("com.apple.dock")?;
        match find_tile(&snapshot, "persistent-others", &path)? {
            None => {
                crate::runner::require_command_at(
                    "dockutil",
                    crate::paths::environment().home_dir()?,
                )?;
                let label = path
                    .file_name()
                    .and_then(std::ffi::OsStr::to_str)
                    .context("Dock folder needs a UTF-8 label")?;
                super::native::identifier(label, "Dock folder label")?;
                ensure_unused_label(&snapshot, label)?;
                Ok(ResourceState::Absent)
            }
            Some(tile) if self.matches(&tile)? => Ok(self.desired_state()),
            Some(tile) => {
                crate::runner::require_command_at(
                    "dockutil",
                    crate::paths::environment().home_dir()?,
                )?;
                unique_label(&snapshot, &tile)?;
                Ok(ResourceState::Modified {
                    from: format!("{:?}", tile.data),
                    to: format!("{}/{}/{}", self.view, self.display, self.sort),
                })
            }
        }
    }
    fn desired_state(&self) -> ResourceState {
        ResourceState::Present {
            details: Some(format!("{}/{}/{}", self.view, self.display, self.sort)),
        }
    }
    fn apply(&self, ctx: &mut ApplyContext) -> Result<ApplyResult> {
        if ctx.dry_run {
            return Ok(ApplyResult::Skipped {
                reason: "Dry run".into(),
            });
        }
        self.options()?;
        let path = desired_path(&self.path)?;
        let snapshot = super::native::preferences("com.apple.dock")?;
        let tile = find_tile(&snapshot, "persistent-others", &path)?;
        let label = if let Some(tile) = &tile {
            if self.matches(tile)? {
                return Ok(ApplyResult::NoChange);
            }
            unique_label(&snapshot, tile)?
        } else {
            let label = path
                .file_name()
                .and_then(std::ffi::OsStr::to_str)
                .context("Dock folder needs a UTF-8 label")?;
            super::native::identifier(label, "Dock folder label")?;
            ensure_unused_label(&snapshot, label)?;
            label.to_string()
        };
        let mut args: Vec<OsString> = vec![
            "--add".into(),
            path.into_os_string(),
            "--no-restart".into(),
            "--section".into(),
            "others".into(),
            "--type".into(),
            "directory".into(),
            "--label".into(),
            label.clone().into(),
            "--view".into(),
            self.view.clone().into(),
            "--display".into(),
            self.display.clone().into(),
            "--sort".into(),
            self.sort.clone().into(),
        ];
        if tile.is_some() {
            args.extend(["--replacing".into(), label.into()]);
        }
        execute(args)?;
        Ok(if tile.is_some() {
            ApplyResult::Modified
        } else {
            ApplyResult::Created
        })
    }
    fn can_parallelize(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(apps: &str, recent: &str, others: &str) -> Dictionary {
        Value::from_reader_xml(format!("<plist><dict><key>persistent-apps</key><array>{apps}</array><key>recent-apps</key><array>{recent}</array><key>persistent-others</key><array>{others}</array></dict></plist>").as_bytes()).unwrap().into_dictionary().unwrap()
    }

    #[test]
    fn selectors_include_recent_bundle_and_decoded_url_identities() {
        let own =
            "<dict><key>tile-data</key><dict><key>file-label</key><string>A</string></dict></dict>";
        for fields in [
            "<key>file-label</key><string>A</string>",
            "<key>bundle-identifier</key><string>A</string>",
            "<key>url</key><dict><key>_CFURLString</key><string>%41</string></dict>",
        ] {
            let recent = format!("<dict><key>tile-data</key><dict>{fields}</dict></dict>");
            assert_eq!(selector_count(&snapshot(own, &recent, ""), "A").unwrap(), 2);
        }
        let urls = "<dict><key>tile-data</key><dict><key>url</key><dict><key>_CFURLString</key><string>file:///%41</string></dict></dict></dict>";
        assert_eq!(selector_count(&snapshot("", "", urls), "/A").unwrap(), 1);
        let unicode = "<dict><key>tile-data</key><dict><key>file-label</key><string>\u{212a}</string></dict></dict>";
        assert_eq!(selector_count(&snapshot("", unicode, ""), "K").unwrap(), 1);
        let own =
            "<dict><key>tile-data</key><dict><key>file-label</key><string>K</string></dict></dict>";
        assert_eq!(selector_count(&snapshot(own, unicode, ""), "K").unwrap(), 2);
    }

    #[test]
    #[cfg(unix)]
    fn app_aliases_use_verified_filesystem_identity() {
        let temp = tempfile::TempDir::new().unwrap();
        let real = temp.path().join("Real.app");
        std::fs::create_dir(&real).unwrap();
        let alias = temp.path().join("Alias.app");
        std::os::unix::fs::symlink(&real, &alias).unwrap();
        let url = url::Url::from_directory_path(&real).unwrap();
        let app = format!(
            "<dict><key>tile-data</key><dict><key>file-label</key><string>Real</string><key>file-data</key><dict><key>_CFURLString</key><string>{url}</string><key>_CFURLStringType</key><integer>15</integer></dict></dict></dict>"
        );
        let layout = DockApps {
            paths: vec![alias.to_str().unwrap().into()],
        };
        assert!(
            DockApps::is_satisfied(&snapshot(&app, "", ""), &layout.expanded().unwrap()).unwrap()
        );
        let plain = temp.path().join("plain-directory");
        std::fs::create_dir(&plain).unwrap();
        assert!(
            DockApps {
                paths: vec![plain.to_str().unwrap().into()]
            }
            .expanded()
            .is_err()
        );
    }

    #[test]
    fn folder_options_require_complete_typed_values() {
        let mut data = Dictionary::new();
        data.insert("showas".into(), Value::Integer(2.into()));
        data.insert("displayas".into(), Value::Integer(0.into()));
        data.insert("arrangement".into(), Value::Integer(2.into()));
        let folder = DockFolder::new("/fixture");
        assert!(
            folder
                .matches(&Tile {
                    position: 1,
                    data: &data
                })
                .unwrap()
        );
        data.insert("showas".into(), Value::Integer(3.into()));
        assert!(
            !folder
                .matches(&Tile {
                    position: 1,
                    data: &data
                })
                .unwrap()
        );
        data.remove("showas");
        assert!(
            folder
                .matches(&Tile {
                    position: 1,
                    data: &data
                })
                .is_err()
        );
    }
}
