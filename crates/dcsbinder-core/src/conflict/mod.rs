//! Group `Active` scanned files by `(install_root, aircraft, subtype, device_name)`
//! and surface groups with >1 distinct GUID as conflicts.
//!
//! Per the plan, v1 does **not** automatically fuzzy-match device names —
//! `L-VPC Rotor TCS` and `LEFT VPC Rotor TCS` stay as separate devices until
//! the user explicitly links them via the alias map (M5).

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::device::guid::Guid;
use crate::scanner::{FileStatus, ScannedFile, Subtype};

/// One detected conflict: two or more `Active` files in the same install /
/// aircraft / subtype share a device name but have different GUIDs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    pub install_root: PathBuf,
    pub aircraft: String,
    pub subtype: Subtype,
    pub device_name: String,
    /// One entry per distinct GUID, sorted by GUID for deterministic output.
    pub candidates: Vec<Candidate>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub guid: String,
    pub path: PathBuf,
}

/// A single-candidate device whose binding GUID does not match any live device,
/// while a live device with the same name exists. The user almost certainly
/// wants the binding remapped under the live GUID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Orphan {
    pub install_root: PathBuf,
    pub aircraft: String,
    pub subtype: Subtype,
    pub device_name: String,
    pub stale_guid: String,
    pub stale_path: PathBuf,
    pub live_guid: String,
}

/// Find every `(install, aircraft, subtype, device_name)` group with exactly
/// one Active file whose GUID is **not** the live device's instance GUID,
/// but where a live device with the same product name exists.
///
/// `live_devices` is the `(product_name, instance_guid_dcs_string)` list from
/// [`crate::device::enumerate`].
#[must_use]
pub fn detect_orphans(files: &[ScannedFile], live_devices: &[(String, Guid)]) -> Vec<Orphan> {
    let mut groups: BTreeMap<GroupKey, Vec<Candidate>> = BTreeMap::new();
    for file in files {
        let FileStatus::Active { device_name, guid } = &file.status else {
            continue;
        };
        let Some(subtype) = file.subtype else {
            continue;
        };
        let key = GroupKey {
            install_root: file.install_root.clone(),
            aircraft: file.aircraft.clone(),
            subtype,
            device_name: device_name.clone(),
        };
        groups.entry(key).or_default().push(Candidate {
            guid: guid.clone(),
            path: file.path.clone(),
        });
    }

    let mut orphans: Vec<Orphan> = Vec::new();
    for (key, candidates) in groups {
        if candidates.len() != 1 {
            continue; // 0 = impossible here; >1 = a conflict, handled separately.
        }
        let cand = &candidates[0];
        let cand_guid_canonical =
            Guid::parse_dcs(&format!("{{{}}}", cand.guid)).map(Guid::to_dcs_string);
        let Ok(cand_canonical) = cand_guid_canonical else {
            continue;
        };
        // Is there a live device with this product name whose GUID doesn't
        // equal the candidate's GUID?
        if let Some((_, live_guid)) = live_devices
            .iter()
            .find(|(name, _)| name == &key.device_name)
        {
            let live_canonical = live_guid.to_dcs_string();
            if live_canonical != cand_canonical {
                orphans.push(Orphan {
                    install_root: key.install_root,
                    aircraft: key.aircraft,
                    subtype: key.subtype,
                    device_name: key.device_name,
                    stale_guid: cand.guid.clone(),
                    stale_path: cand.path.clone(),
                    live_guid: live_canonical,
                });
            }
        }
    }
    orphans.sort_by(|a, b| {
        a.install_root
            .cmp(&b.install_root)
            .then_with(|| a.aircraft.cmp(&b.aircraft))
            .then_with(|| a.subtype.as_str().cmp(b.subtype.as_str()))
            .then_with(|| a.device_name.cmp(&b.device_name))
    });
    orphans
}

/// Find every device-name+GUID conflict in `files`. Output is sorted by
/// `(install, aircraft, subtype, device_name)` for deterministic reporting.
#[must_use]
pub fn detect(files: &[ScannedFile]) -> Vec<Conflict> {
    // (install, aircraft, subtype, device_name) -> [(guid, path), ...]
    let mut groups: BTreeMap<GroupKey, Vec<Candidate>> = BTreeMap::new();

    for file in files {
        let FileStatus::Active { device_name, guid } = &file.status else {
            continue;
        };
        let Some(subtype) = file.subtype else {
            continue;
        };
        let key = GroupKey {
            install_root: file.install_root.clone(),
            aircraft: file.aircraft.clone(),
            subtype,
            device_name: device_name.clone(),
        };
        groups.entry(key).or_default().push(Candidate {
            guid: guid.clone(),
            path: file.path.clone(),
        });
    }

    let mut conflicts: Vec<Conflict> = groups
        .into_iter()
        .filter_map(|(key, mut candidates)| {
            // Only conflicts: >1 distinct GUID.
            let distinct: std::collections::HashSet<_> =
                candidates.iter().map(|c| c.guid.as_str()).collect();
            if distinct.len() < 2 {
                return None;
            }
            candidates.sort_by(|a, b| a.guid.cmp(&b.guid));
            Some(Conflict {
                install_root: key.install_root,
                aircraft: key.aircraft,
                subtype: key.subtype,
                device_name: key.device_name,
                candidates,
            })
        })
        .collect();

    // BTreeMap iterates in key order already, but be explicit.
    conflicts.sort_by(|a, b| {
        a.install_root
            .cmp(&b.install_root)
            .then_with(|| a.aircraft.cmp(&b.aircraft))
            .then_with(|| a.subtype.as_str().cmp(b.subtype.as_str()))
            .then_with(|| a.device_name.cmp(&b.device_name))
    });
    conflicts
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct GroupKey {
    install_root: PathBuf,
    aircraft: String,
    // Order Subtype manually since it's Copy + Eq but not Ord by default.
    // BTreeMap requires Ord — we derive it via the string repr.
    subtype: Subtype,
    device_name: String,
}

// Implement Ord for Subtype via its dir-name string.
impl PartialOrd for Subtype {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Subtype {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.as_str().cmp(other.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn detects_mfdleft_conflict_in_fixtures() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures");
        let files = crate::scanner::scan(&root);
        let conflicts = detect(&files);

        let mfd_left: Vec<_> = conflicts
            .iter()
            .filter(|c| c.device_name == "MFDLeft")
            .collect();
        assert_eq!(
            mfd_left.len(),
            1,
            "expected exactly one MFDLeft conflict, got {} ({mfd_left:#?})",
            mfd_left.len()
        );
        let c = mfd_left[0];
        assert_eq!(c.aircraft, "A-10C II");
        assert_eq!(c.subtype, Subtype::Joystick);
        assert_eq!(c.candidates.len(), 2);
        let guids: Vec<_> = c.candidates.iter().map(|cand| cand.guid.as_str()).collect();
        assert!(guids.contains(&"4E50F3B0-2309-11ee-8015-444553540000"));
        assert!(guids.contains(&"CD3E4960-E0D2-11ef-8014-444553540000"));
    }

    #[test]
    fn single_guid_device_is_not_a_conflict() {
        // Warthog only has one GUID in fixtures, so no conflict on it.
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures");
        let files = crate::scanner::scan(&root);
        let conflicts = detect(&files);

        assert!(
            !conflicts
                .iter()
                .any(|c| c.device_name.contains("HOTAS Warthog")),
            "Warthog should not be flagged as a conflict (only one GUID in corpus)"
        );
    }
}
