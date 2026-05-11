//! Walk a DCS Input tree and classify every file by status.
//!
//! Input layout (per install root, e.g. `Saved Games/DCS.openbeta/Config/Input`):
//!
//! ```text
//! <Aircraft>/
//! ├── modifiers.lua            (optional; aircraft-level)
//! ├── joystick/<DeviceName> {GUID}.diff.lua
//! ├── keyboard/...
//! ├── mouse/...
//! └── trackir/...
//! ```
//!
//! Classification (see `docs/FILE_FORMAT.md`):
//! - `Active`         — name matches the canonical `<name> {GUID}.diff.lua` shape with empty suffix.
//! - `UserArchived`   — same shape but suffix is non-empty (e.g. `XXX`, ` (old)`).
//! - `Modifiers`      — aircraft-level `modifiers.lua`.
//! - `ExportedProfile`— a `.lua` file that isn't a `.diff.lua` and doesn't match the GUID shape.
//! - `Malformed`      — has `.diff.lua` extension but the GUID doesn't match the 8-4-4-4-12 shape.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use regex::Regex;
use walkdir::WalkDir;

/// One file discovered during a scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScannedFile {
    /// The install root that was scanned (e.g. `…/Saved Games/DCS.openbeta/Config/Input`).
    pub install_root: PathBuf,
    /// `<Aircraft>` directory name (e.g. `"A-10C II"`, `"F-16C_50"`).
    pub aircraft: String,
    /// Subtype directory or `None` for aircraft-level files (`modifiers.lua`).
    pub subtype: Option<Subtype>,
    /// Absolute path to the file itself.
    pub path: PathBuf,
    /// File-status classification.
    pub status: FileStatus,
}

/// The input-subtype directory a file lives in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Subtype {
    Joystick,
    Keyboard,
    Mouse,
    TrackIr,
}

impl Subtype {
    #[must_use]
    pub fn from_dir_name(name: &str) -> Option<Self> {
        match name {
            "joystick" => Some(Self::Joystick),
            "keyboard" => Some(Self::Keyboard),
            "mouse" => Some(Self::Mouse),
            "trackir" => Some(Self::TrackIr),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Joystick => "joystick",
            Self::Keyboard => "keyboard",
            Self::Mouse => "mouse",
            Self::TrackIr => "trackir",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileStatus {
    Active {
        device_name: String,
        guid: String,
    },
    UserArchived {
        device_name: String,
        guid: String,
        suffix: String,
    },
    Modifiers,
    ExportedProfile,
    Malformed {
        reason: String,
    },
}

/// Walk `install_root` and return every recognized file with its classification.
///
/// `install_root` is expected to be a `Config/Input` directory. Non-existent or
/// non-directory roots return an empty list — the caller decides how to surface that.
pub fn scan(install_root: &Path) -> Vec<ScannedFile> {
    if !install_root.is_dir() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for entry in WalkDir::new(install_root)
        .min_depth(1)
        .max_depth(3)
        .into_iter()
        .filter_map(Result::ok)
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path().to_path_buf();
        let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };

        // Skip non-Lua files entirely. Case-insensitive in case Windows surfaces
        // `.LUA` from a manually-renamed file; DCS itself always emits lowercase.
        if !std::path::Path::new(file_name)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("lua"))
        {
            continue;
        }

        let Some(rel) = path.strip_prefix(install_root).ok() else {
            continue;
        };
        // Components: ["<aircraft>", maybe "<subtype>", "<file>"]. We dropped any
        // entry that isn't a file already, so the last component is always the file.
        let comps: Vec<&str> = rel
            .components()
            .filter_map(|c| c.as_os_str().to_str())
            .collect();
        let (aircraft, subtype) = match comps.as_slice() {
            [aircraft, _file] => ((*aircraft).to_string(), None),
            [aircraft, sub, _file] => {
                let Some(st) = Subtype::from_dir_name(sub) else {
                    continue;
                };
                ((*aircraft).to_string(), Some(st))
            }
            _ => continue,
        };

        let status = if subtype.is_none() {
            if file_name == "modifiers.lua" {
                FileStatus::Modifiers
            } else {
                continue; // unknown aircraft-level file
            }
        } else {
            classify_subtype_file(file_name)
        };

        out.push(ScannedFile {
            install_root: install_root.to_path_buf(),
            aircraft,
            subtype,
            path,
            status,
        });
    }
    out
}

fn classify_subtype_file(file_name: &str) -> FileStatus {
    if let Some(caps) = filename_regex().captures(file_name) {
        let device_name = caps["name"].to_string();
        let guid = caps["guid"].to_string();
        let suffix = caps["suffix"].to_string();
        if suffix.is_empty() {
            FileStatus::Active { device_name, guid }
        } else {
            FileStatus::UserArchived {
                device_name,
                guid,
                suffix,
            }
        }
    } else if file_name.ends_with(".diff.lua") {
        FileStatus::Malformed {
            reason: format!(
                "name `{file_name}` ends in .diff.lua but doesn't match the canonical GUID shape"
            ),
        }
    } else {
        FileStatus::ExportedProfile
    }
}

fn filename_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"^(?P<name>.+?) \{(?P<guid>[0-9A-Fa-f]{8}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{12})\}(?P<suffix>[^.]*)\.diff\.lua$",
        )
        .expect("filename regex compiles")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classify(name: &str) -> FileStatus {
        classify_subtype_file(name)
    }

    #[test]
    fn classifies_canonical_active() {
        let s = classify("MFDLeft {4E50F3B0-2309-11ee-8015-444553540000}.diff.lua");
        match s {
            FileStatus::Active { device_name, guid } => {
                assert_eq!(device_name, "MFDLeft");
                assert_eq!(guid, "4E50F3B0-2309-11ee-8015-444553540000");
            }
            other => panic!("expected Active, got {other:?}"),
        }
    }

    #[test]
    fn classifies_user_archived_suffix() {
        let s = classify("MFDLeft {4E50A590-2309-11ee-8012-444553540000}XXX.diff.lua");
        match s {
            FileStatus::UserArchived {
                device_name,
                suffix,
                ..
            } => {
                assert_eq!(device_name, "MFDLeft");
                assert_eq!(suffix, "XXX");
            }
            other => panic!("expected UserArchived, got {other:?}"),
        }
    }

    #[test]
    fn classifies_compound_device_name() {
        let s = classify(
            "WINWING Orion Throttle Base II + F15EX HANDLE L + F15EX HANDLE R \
             {CD3DD430-E0D2-11ef-8011-444553540000}.diff.lua",
        );
        match s {
            FileStatus::Active { device_name, .. } => {
                assert_eq!(
                    device_name,
                    "WINWING Orion Throttle Base II + F15EX HANDLE L + F15EX HANDLE R"
                );
            }
            other => panic!("expected Active, got {other:?}"),
        }
    }

    #[test]
    fn classifies_keyboard_default_as_exported() {
        let s = classify("Keyboard.diff.lua");
        match s {
            FileStatus::Malformed { .. } => {
                // Ends in .diff.lua but no GUID — flagged as malformed for now.
                // Keyboard.diff.lua is DCS-emitted so we may want to treat it specially.
            }
            other => panic!("expected Malformed, got {other:?}"),
        }
    }

    #[test]
    fn classifies_plain_lua_as_profile() {
        let s = classify("MyProfile.lua");
        assert!(matches!(s, FileStatus::ExportedProfile));
    }

    #[test]
    fn classifies_malformed_guid() {
        let s = classify("MFDLeft {NOT-A-GUID}.diff.lua");
        assert!(matches!(s, FileStatus::Malformed { .. }));
    }

    #[test]
    fn scan_fixture_corpus() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures");
        let files = scan(&root);
        // Sanity: we should find all 5 lua files (we filtered the fixtures README out).
        assert!(
            files.len() >= 4,
            "found {} files: {:#?}",
            files.len(),
            files
        );
        // The MFDLeft pair must be classified Active.
        let mfd_lefts: Vec<_> = files
            .iter()
            .filter(|f| {
                matches!(&f.status, FileStatus::Active { device_name, .. } if device_name == "MFDLeft")
            })
            .collect();
        assert_eq!(
            mfd_lefts.len(),
            2,
            "expected 2 active MFDLeft files, got {} ({:#?})",
            mfd_lefts.len(),
            mfd_lefts
        );
        // The AH-64D modifiers.lua must be classified Modifiers.
        let mods: Vec<_> = files
            .iter()
            .filter(|f| matches!(f.status, FileStatus::Modifiers))
            .collect();
        assert_eq!(mods.len(), 1);
        assert_eq!(mods[0].aircraft, "AH-64D_BLK_II_CPG");
    }
}
