//! Live-device-vs-filename correlation test (the load-bearing M2 test).
//!
//! Runs `DirectInput` enumeration on the host, scans the project owner's real DCS
//! install (auto-discovered), and asserts that at least one live instance GUID
//! matches at least one DCS filename's GUID.
//!
//! `#[ignore]` by default because it needs both real hardware and a real DCS
//! install. Run locally via `cargo test --test live_correlation -- --ignored`.

#![cfg(windows)]

use std::collections::HashSet;

use dcsbinder_core::{config, device, scanner};

#[test]
#[ignore = "requires attached controllers + a real DCS install"]
fn live_device_guid_matches_a_dcs_filename() {
    let installs = config::discover_installs();
    assert!(
        !installs.is_empty(),
        "no DCS install discovered under Saved Games\\DCS*"
    );

    let live = device::enumerate().expect("DirectInput enumeration succeeded");
    assert!(!live.is_empty(), "no game controllers attached");

    let live_guids: HashSet<String> = live
        .iter()
        .map(|d| d.instance_guid.to_dcs_string())
        .collect();

    let mut any_match = false;
    let mut total_active = 0usize;
    for install in &installs {
        let files = scanner::scan(&install.input_root);
        for f in &files {
            if let scanner::FileStatus::Active { guid, .. } = &f.status {
                total_active += 1;
                if let Ok(parsed) = device::guid::Guid::parse_dcs(&format!("{{{guid}}}")) {
                    if live_guids.contains(&parsed.to_dcs_string()) {
                        any_match = true;
                    }
                }
            }
        }
    }

    assert!(
        any_match,
        "no live DirectInput instance GUID matched any of the {total_active} \
         Active DCS bind files. Live device count: {}. This means either the \
         DCS bind files are all stale (user just had a GUID reassignment, which \
         is the case DCSBinder solves) or there's a format-conversion bug.",
        live.len()
    );
}
