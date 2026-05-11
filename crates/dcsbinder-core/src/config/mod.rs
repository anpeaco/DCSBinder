//! Configuration & install discovery.
//!
//! Locates `Saved Games/DCS*/Config/Input` roots, exposes the
//! `%APPDATA%/DCSBinder/` paths the rest of the app uses, and answers
//! "is DCS running right now?" via `sysinfo`.

use std::path::{Path, PathBuf};

/// One discovered DCS install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DcsInstall {
    pub flavor: DcsFlavor,
    /// Root of the install's user-config tree (`Saved Games\<flavor>\`).
    pub saved_games_root: PathBuf,
    /// Absolute path to `Config\Input\`, the directory we scan.
    pub input_root: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DcsFlavor {
    Stable,
    OpenBeta,
    ServerBeta,
}

impl DcsFlavor {
    /// Subdirectory under `Saved Games` for this flavor.
    #[must_use]
    pub fn saved_games_dir(self) -> &'static str {
        match self {
            Self::Stable => "DCS",
            Self::OpenBeta => "DCS.openbeta",
            Self::ServerBeta => "DCS.dcs_serverbeta",
        }
    }
}

/// Discover every DCS install under `%USERPROFILE%\Saved Games\`.
///
/// Returns installs whose `Config\Input\` directory actually exists. Does not
/// fail; if `Saved Games` is missing, returns an empty `Vec`.
#[must_use]
pub fn discover_installs() -> Vec<DcsInstall> {
    let Some(saved_games) = saved_games_dir() else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for flavor in [
        DcsFlavor::Stable,
        DcsFlavor::OpenBeta,
        DcsFlavor::ServerBeta,
    ] {
        let root = saved_games.join(flavor.saved_games_dir());
        let input_root = root.join("Config").join("Input");
        if input_root.is_dir() {
            out.push(DcsInstall {
                flavor,
                saved_games_root: root,
                input_root,
            });
        }
    }
    out
}

/// `%USERPROFILE%\Saved Games`, resolved via the platform's "user profile" path.
///
/// Windows hides `Saved Games` from the standard `directories` crate (it's a
/// known folder, not an XDG one), so we look it up directly off `USERPROFILE`.
#[must_use]
pub fn saved_games_dir() -> Option<PathBuf> {
    // `USERPROFILE` is always set in a Windows user session and is preserved
    // through OneDrive sync; canonicalize-on-use elsewhere will detect that case.
    let user_profile = std::env::var_os("USERPROFILE")?;
    let candidate = Path::new(&user_profile).join("Saved Games");
    candidate.is_dir().then_some(candidate)
}

/// `%APPDATA%\DCSBinder\` — where backups, history, settings, and aliases live.
#[must_use]
pub fn app_data_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "DCSBinder").map(|p| p.data_dir().to_path_buf())
}

/// Returns the PID of any running `DCS.exe` process, or `None` if not running.
///
/// Conservative match: looks for the executable basename `DCS.exe` (case-insensitive),
/// which covers both stable and openbeta installs.
#[must_use]
pub fn dcs_running() -> Option<u32> {
    use sysinfo::System;

    let mut sys = System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, false);

    for (pid, process) in sys.processes() {
        let name = process.name().to_string_lossy();
        if name.eq_ignore_ascii_case("DCS.exe") {
            return Some(pid.as_u32());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flavor_dir_names() {
        assert_eq!(DcsFlavor::Stable.saved_games_dir(), "DCS");
        assert_eq!(DcsFlavor::OpenBeta.saved_games_dir(), "DCS.openbeta");
        assert_eq!(
            DcsFlavor::ServerBeta.saved_games_dir(),
            "DCS.dcs_serverbeta"
        );
    }

    #[test]
    fn discover_returns_vec() {
        // Smoke test: must not panic and must not return junk paths.
        let installs = discover_installs();
        for i in &installs {
            assert!(i.input_root.is_dir(), "advertised input_root must exist");
            assert!(i.input_root.ends_with("Input"));
        }
    }
}
