//! Integration tests for the M3 remap engine.
//!
//! Each test builds a synthetic DCS Input tree under a `TempDir`, runs the full
//! `plan -> execute` path, and asserts the resulting filesystem state. Then it
//! runs `undo` and asserts the state is restored byte-equal to the original.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use dcsbinder_core::device::guid::Guid;
use dcsbinder_core::remap::{self, Manifest, Mutation};
use dcsbinder_core::scanner::{self, Subtype};

const OLD_GUID_STR: &str = "{4E50F3B0-2309-11ee-8015-444553540000}";
const NEW_GUID_STR: &str = "{CD3E4960-E0D2-11ef-8014-444553540000}";

/// Create a synthetic DCS Input tree under `root`. Returns the `install_root` path.
fn build_synthetic_install(root: &Path, aircrafts: &[&str], devices: &[(&str, &str, &str)]) {
    // devices: (aircraft, filename, content). filename includes "{GUID}.diff.lua".
    for aircraft in aircrafts {
        fs::create_dir_all(root.join(aircraft).join("joystick")).unwrap();
    }
    for (aircraft, filename, content) in devices {
        let path = root.join(aircraft).join("joystick").join(filename);
        fs::write(&path, content).unwrap_or_else(|e| {
            panic!("write {} failed: {e}", path.display());
        });
    }
}

fn list_dir_sorted(dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .filter_map(|e| e.file_name().to_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

#[test]
fn plan_and_execute_remap_single_aircraft() {
    let temp = tempfile::tempdir().unwrap();
    let install_root = temp.path().join("Input");
    fs::create_dir_all(&install_root).unwrap();

    let old_content = b"-- old bindings (rich)\nlocal diff = { ['axisDiffs'] = {}, ['keyDiffs'] = {} }\nreturn diff";
    let new_content = b"-- new bindings (sparse)\nlocal diff = { ['axisDiffs'] = {}, ['keyDiffs'] = {} }\nreturn diff";

    build_synthetic_install(
        &install_root,
        &["A-10C II"],
        &[
            (
                "A-10C II",
                &format!("MFDLeft {OLD_GUID_STR}.diff.lua"),
                std::str::from_utf8(old_content).unwrap(),
            ),
            (
                "A-10C II",
                &format!("MFDLeft {NEW_GUID_STR}.diff.lua"),
                std::str::from_utf8(new_content).unwrap(),
            ),
        ],
    );

    let backup_root = temp.path().join("backups");
    let files = scanner::scan(&install_root);

    let source = Guid::parse_dcs(OLD_GUID_STR).unwrap();
    let target = Guid::parse_dcs(NEW_GUID_STR).unwrap();

    let manifest = remap::plan(
        &install_root,
        "MFDLeft",
        Subtype::Joystick,
        &source,
        &target,
        &files,
        &backup_root,
    )
    .expect("plan succeeds");

    // Manifest sanity.
    assert_eq!(manifest.device_name, "MFDLeft");
    assert_eq!(
        manifest.source_guid,
        "{4E50F3B0-2309-11EE-8015-444553540000}"
    );
    assert_eq!(
        manifest.target_guid,
        "{CD3E4960-E0D2-11EF-8014-444553540000}"
    );
    assert_eq!(manifest.backups.len(), 2, "should back up both files");
    assert_eq!(manifest.mutations.len(), 2, "WriteFile + MoveFile");

    // Execute.
    let done_marker = remap::execute(&manifest, false).expect("execute succeeds");
    assert!(done_marker.exists(), ".done marker should exist");
    assert!(Manifest::path_in(&manifest.backup_dir).exists());

    // Final filesystem state in the original tree.
    // Note: `to_dcs_string` emits uppercase; the existing lowercase file is
    // overwritten case-insensitively on Windows. Check by case-insensitive name.
    let joystick_dir = install_root.join("A-10C II").join("joystick");
    let listing = list_dir_sorted(&joystick_dir);
    assert_eq!(
        listing.len(),
        1,
        "exactly one file should remain, got {listing:?}"
    );
    let remaining = &listing[0];
    let expected_remaining = format!("MFDLeft {NEW_GUID_STR}.diff.lua");
    assert!(
        remaining.eq_ignore_ascii_case(&expected_remaining),
        "old GUID file should be archived; only new GUID file remains. got: {remaining}"
    );

    // The target file now contains the OLD bytes (the chosen source-of-truth).
    let target_path = joystick_dir.join(remaining);
    let target_bytes = fs::read(&target_path).unwrap();
    assert_eq!(
        target_bytes, old_content,
        "target file should contain source-of-truth bytes"
    );

    // The OLD file is now archived inside the backup folder.
    let archived_old = manifest
        .backup_dir
        .join("archived")
        .join("A-10C II")
        .join("joystick")
        .join(format!("MFDLeft {OLD_GUID_STR}.diff.lua"));
    assert!(archived_old.is_file(), "OLD file should be archived");
    let archived_bytes = fs::read(&archived_old).unwrap();
    assert_eq!(archived_bytes, old_content);
}

#[test]
fn undo_restores_original_state_byte_equal() {
    let temp = tempfile::tempdir().unwrap();
    let install_root = temp.path().join("Input");
    fs::create_dir_all(&install_root).unwrap();

    let old_content = b"-- OLD\nlocal diff = {}\nreturn diff";
    let new_content = b"-- NEW\nlocal diff = {}\nreturn diff";

    build_synthetic_install(
        &install_root,
        &["A-10C II", "F-16C_50"],
        &[
            (
                "A-10C II",
                &format!("MFDLeft {OLD_GUID_STR}.diff.lua"),
                std::str::from_utf8(old_content).unwrap(),
            ),
            (
                "A-10C II",
                &format!("MFDLeft {NEW_GUID_STR}.diff.lua"),
                std::str::from_utf8(new_content).unwrap(),
            ),
            (
                "F-16C_50",
                &format!("MFDLeft {OLD_GUID_STR}.diff.lua"),
                std::str::from_utf8(old_content).unwrap(),
            ),
            (
                "F-16C_50",
                &format!("MFDLeft {NEW_GUID_STR}.diff.lua"),
                std::str::from_utf8(new_content).unwrap(),
            ),
        ],
    );

    // Snapshot the original tree.
    let original = snapshot_tree(&install_root);

    let backup_root = temp.path().join("backups");
    let files = scanner::scan(&install_root);
    let source = Guid::parse_dcs(OLD_GUID_STR).unwrap();
    let target = Guid::parse_dcs(NEW_GUID_STR).unwrap();

    let manifest = remap::plan(
        &install_root,
        "MFDLeft",
        Subtype::Joystick,
        &source,
        &target,
        &files,
        &backup_root,
    )
    .unwrap();

    let _done = remap::execute(&manifest, false).unwrap();

    // Verify mutated.
    let after_execute = snapshot_tree(&install_root);
    assert_ne!(original, after_execute, "tree should have changed");

    // Undo.
    let manifest_path = Manifest::path_in(&manifest.backup_dir);
    remap::undo(&manifest_path).expect("undo succeeds");

    let restored = snapshot_tree(&install_root);
    assert_eq!(
        original, restored,
        "undo must restore the install tree byte-equal"
    );
}

#[test]
fn refuses_when_source_equals_target() {
    let temp = tempfile::tempdir().unwrap();
    let install_root = temp.path().join("Input");
    fs::create_dir_all(install_root.join("A-10C II").join("joystick")).unwrap();
    let backup_root = temp.path().join("backups");
    let files = scanner::scan(&install_root);
    let g = Guid::parse_dcs(OLD_GUID_STR).unwrap();
    let err = remap::plan(
        &install_root,
        "MFDLeft",
        Subtype::Joystick,
        &g,
        &g,
        &files,
        &backup_root,
    )
    .expect_err("plan should reject source==target");
    let msg = format!("{err}");
    assert!(msg.contains("nothing to do"), "got: {msg}");
}

#[test]
fn rewrites_modifiers_lua_when_guid_referenced() {
    let temp = tempfile::tempdir().unwrap();
    let install_root = temp.path().join("Input");
    fs::create_dir_all(install_root.join("A-10C II").join("joystick")).unwrap();

    let mfd_old = format!("MFDLeft {OLD_GUID_STR}.diff.lua");
    fs::write(
        install_root
            .join("A-10C II")
            .join("joystick")
            .join(&mfd_old),
        b"-- OLD\nlocal diff = {}\nreturn diff",
    )
    .unwrap();
    let mfd_new = format!("MFDLeft {NEW_GUID_STR}.diff.lua");
    fs::write(
        install_root
            .join("A-10C II")
            .join("joystick")
            .join(&mfd_new),
        b"-- NEW\nlocal diff = {}\nreturn diff",
    )
    .unwrap();

    // modifiers.lua references the OLD GUID twice (different switches).
    let modifiers_path = install_root.join("A-10C II").join("modifiers.lua");
    let modifiers_content = format!(
        "local modifiers = {{\n\
         \t[\"JOY_BTN8\"] = {{ [\"device\"] = \"MFDLeft {OLD_GUID_STR}\" }},\n\
         \t[\"JOY_BTN9\"] = {{ [\"device\"] = \"MFDLeft {OLD_GUID_STR}\" }},\n\
         }}\nreturn modifiers"
    );
    fs::write(&modifiers_path, &modifiers_content).unwrap();

    let backup_root = temp.path().join("backups");
    let files = scanner::scan(&install_root);
    let source = Guid::parse_dcs(OLD_GUID_STR).unwrap();
    let target = Guid::parse_dcs(NEW_GUID_STR).unwrap();

    let manifest = remap::plan(
        &install_root,
        "MFDLeft",
        Subtype::Joystick,
        &source,
        &target,
        &files,
        &backup_root,
    )
    .unwrap();

    let string_replaces: Vec<_> = manifest
        .mutations
        .iter()
        .filter(|m| matches!(m, Mutation::StringReplace { .. }))
        .collect();
    assert_eq!(
        string_replaces.len(),
        1,
        "exactly one modifiers.lua rewrite"
    );
    if let Mutation::StringReplace {
        expected_replacements,
        ..
    } = string_replaces[0]
    {
        assert_eq!(*expected_replacements, 2, "two GUID occurrences expected");
    }

    remap::execute(&manifest, false).unwrap();

    let new_modifiers = fs::read_to_string(&modifiers_path).unwrap();
    // Compare case-insensitively: the replace writes canonical (uppercase)
    // form regardless of the original casing.
    let new_upper = new_modifiers.to_ascii_uppercase();
    assert!(
        new_upper.contains(&NEW_GUID_STR.to_ascii_uppercase()),
        "modifiers.lua should now reference the NEW GUID. got:\n{new_modifiers}"
    );
    assert!(
        !new_upper.contains(&OLD_GUID_STR.to_ascii_uppercase()),
        "OLD GUID should be gone from modifiers.lua. got:\n{new_modifiers}"
    );

    // And undo restores it.
    remap::undo(&Manifest::path_in(&manifest.backup_dir)).unwrap();
    let restored = fs::read_to_string(&modifiers_path).unwrap();
    assert_eq!(restored, modifiers_content);
}

#[test]
fn recover_surfaces_un_done_manifests() {
    let temp = tempfile::tempdir().unwrap();
    let backup_root = temp.path().join("backups");

    // Synthesize an un-done manifest dir manually.
    let dir = backup_root.join("test-op");
    fs::create_dir_all(&dir).unwrap();
    let manifest = Manifest {
        version: dcsbinder_core::remap::MANIFEST_VERSION,
        operation_id: "test".into(),
        operation: dcsbinder_core::remap::OperationKind::Remap,
        timestamp: "test".into(),
        backup_dir: dir.clone(),
        install_root: temp.path().to_path_buf(),
        device_name: "Test".into(),
        subtype: "joystick".into(),
        source_guid: OLD_GUID_STR.into(),
        target_guid: NEW_GUID_STR.into(),
        backups: vec![],
        mutations: vec![],
    };
    let json = serde_json::to_vec_pretty(&manifest).unwrap();
    fs::write(Manifest::path_in(&dir), json).unwrap();

    let incomplete = remap::recover(&backup_root).expect("recover ok");
    assert_eq!(incomplete.len(), 1);
    assert_eq!(incomplete[0].manifest.device_name, "Test");

    // Once .done is written, recover stops surfacing it.
    fs::write(Manifest::done_marker_in(&dir), b"done").unwrap();
    let incomplete = remap::recover(&backup_root).unwrap();
    assert_eq!(incomplete.len(), 0);
}

/// Snapshot every file under `root` as path-relative-to-root -> bytes.
fn snapshot_tree(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        for entry in rd.flatten() {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.is_file() {
                let rel = p.strip_prefix(root).unwrap().to_path_buf();
                let bytes = fs::read(&p).unwrap();
                out.insert(rel, bytes);
            }
        }
    }
    out
}
