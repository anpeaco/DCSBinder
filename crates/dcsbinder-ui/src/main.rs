// DCSBinder UI — M4 v1.
//
// Single window app. Auto-discovers DCS installs, scans + enumerates live
// devices on a background thread, then drives the Slint UI from the main
// (event-loop) thread.

// Panic hook (below) logs to %TEMP%\dcsbinder-ui-panic.txt so silent exits are diagnosable.
#![windows_subsystem = "windows"]
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::too_many_lines,
    clippy::needless_pass_by_value
)]

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use dcsbinder_core::{
    config,
    conflict::{detect, detect_orphans, Candidate, Conflict, Orphan},
    device::{self, guid::Guid, LiveDevice},
    remap::{self, Manifest, Mutation},
    scanner::{self, FileStatus, ScannedFile, Subtype},
};
use similar::{ChangeTag, TextDiff};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

slint::include_modules!();

#[derive(Clone)]
struct Item {
    install_root: PathBuf,
    aircraft: String,
    subtype: Subtype,
    device_name: String,
    /// Either two GUIDs (conflict) or one stale + an `orphan_target_guid` (orphan).
    candidates: Vec<Candidate>,
    /// `Some` for orphan rows; `None` for plain conflicts.
    orphan_target_guid: Option<String>,
    /// GUID of the live device matching this name (canonical/uppercase form).
    /// Used by the UI to mark candidates as LIVE.
    live_guid: Option<String>,
}

#[derive(Default, Clone)]
struct AppData {
    installs: Vec<config::DcsInstall>,
    scanned_files: Vec<ScannedFile>,
    live_devices: Vec<LiveDevice>,
    /// Unfiltered items. The UI sees a filtered view derived from `filter`.
    items: Vec<Item>,
    /// Current filter text (case-folded). Empty = no filter.
    filter: String,
    /// Category filter: 0 = all, 1 = conflicts only, 2 = orphans only.
    category: i32,
}

#[derive(Clone)]
struct PendingApply {
    manifest: Manifest,
}

/// Log panics to %TEMP%\dcsbinder-ui-panic.txt. The `windows_subsystem = "windows"`
/// attribute disconnects this binary from any console, so an uncaught panic would
/// otherwise vanish without trace.
fn install_panic_hook() {
    let log_path = std::env::temp_dir().join("dcsbinder-ui-panic.txt");
    std::panic::set_hook(Box::new(move |info| {
        let backtrace = std::backtrace::Backtrace::force_capture();
        let msg = format!(
            "[{}] panic: {info}\nbacktrace:\n{backtrace}\n\n",
            chrono_like_timestamp(),
        );
        let _ = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(&log_path)
            .and_then(|mut f| std::io::Write::write_all(&mut f, msg.as_bytes()));
    }));
}

fn chrono_like_timestamp() -> String {
    // Avoid pulling chrono just for a startup log timestamp.
    use std::time::SystemTime;
    let secs = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    format!("unix={secs}")
}

fn main() -> Result<()> {
    startup_trace("main entered");
    install_panic_hook();
    startup_trace("panic hook installed");
    let app = match App::new() {
        Ok(a) => {
            startup_trace("App::new ok");
            a
        }
        Err(e) => {
            startup_trace(&format!("App::new FAILED: {e:?}"));
            return Err(e.into());
        }
    };
    let state = Arc::new(Mutex::new(AppData::default()));
    let pending = Arc::new(Mutex::new(None::<PendingApply>));

    wire_callbacks(&app, state.clone(), pending.clone());
    startup_trace("callbacks wired");

    trigger_rescan(&app, state.clone());
    startup_trace("first rescan triggered");

    startup_trace("about to call app.run()");
    let run_result = app.run();
    startup_trace(&format!("app.run() returned: {run_result:?}"));
    run_result?;
    Ok(())
}

fn startup_trace(line: &str) {
    // Write to TWO locations so at least one is reachable regardless of
    // %TEMP% / OneDrive policy quirks: the user's TEMP and a fixed
    // %USERPROFILE%\dcsbinder-startup.log next to the user's home dir.
    let temp_log = std::env::temp_dir().join("dcsbinder-ui-startup.txt");
    let home_log = std::env::var_os("USERPROFILE")
        .map(|p| std::path::PathBuf::from(p).join("dcsbinder-startup.log"));

    for target in [Some(temp_log), home_log].into_iter().flatten() {
        let _ = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(&target)
            .and_then(|mut f| {
                use std::io::Write;
                writeln!(f, "[unix={}] {}", chrono_like_timestamp(), line)
            });
    }
}

fn wire_callbacks(
    app: &App,
    state: Arc<Mutex<AppData>>,
    pending: Arc<Mutex<Option<PendingApply>>>,
) {
    let app_state = app.global::<AppState>();

    // rescan
    {
        let weak = app.as_weak();
        let state = state.clone();
        app_state.on_rescan(move || {
            if let Some(app) = weak.upgrade() {
                trigger_rescan(&app, state.clone());
            }
        });
    }

    // select-conflict
    {
        let weak = app.as_weak();
        let state = state.clone();
        app_state.on_select_conflict(move |idx| {
            let Some(app) = weak.upgrade() else { return };
            let st = state.lock().unwrap();
            let app_state = app.global::<AppState>();
            app_state.set_selected_index(idx);

            // Clone the selected item out (so we can drop the lock before
            // I/O-heavy diff computation), then derive everything that needs
            // the full state (aircraft context + affected count) while still
            // holding the lock.
            let filtered = filtered_items(&st);
            let selected_item: Option<Item> = filtered.get(idx as usize).map(|i| (*i).clone());
            let (context, affected_list, affected) = if let Some(item) = selected_item.as_ref() {
                let list = build_affected_aircraft_list(&st, item);
                let count = list.len() as i32;
                (build_aircraft_context(&st, item), list, count)
            } else {
                (Vec::new(), Vec::new(), 0)
            };
            drop(st);

            let (segments, sbs) = if let Some(item) = selected_item.as_ref() {
                compute_diffs(item)
            } else {
                (Vec::new(), Vec::new())
            };
            let inline_changed = segments.iter().filter(|s| s.kind != 0).count() as i32;
            let sbs_changed = sbs
                .iter()
                .filter(|r| r.header_kind != 0 || r.left_kind != 0 || r.right_kind != 0)
                .count() as i32;
            app_state.set_inline_changed_count(inline_changed);
            app_state.set_sbs_changed_count(sbs_changed);

            let cm: Rc<VecModel<AircraftDevice>> = Rc::new(VecModel::from(context));
            app_state.set_aircraft_devices(ModelRc::from(cm));
            let am: Rc<VecModel<AffectedAircraft>> = Rc::new(VecModel::from(affected_list));
            app_state.set_affected_aircraft(ModelRc::from(am));
            app_state.set_affected_aircraft_count(affected);
            let m: Rc<VecModel<DiffSegment>> = Rc::new(VecModel::from(segments));
            app_state.set_diff_segments(ModelRc::from(m));
            let s: Rc<VecModel<SbsRow>> = Rc::new(VecModel::from(sbs));
            app_state.set_sbs_rows(ModelRc::from(s));
            // Reset scroll positions for the new selection.
            app_state.set_inline_viewport_y(0.0);
            app_state.set_sbs_viewport_y(0.0);
        });
    }

    // request-plan
    {
        let weak = app.as_weak();
        let state = state.clone();
        let pending = pending.clone();
        app_state.on_request_plan(
            move |from_guid: SharedString, to_guid: SharedString, scope: i32| {
                let Some(app) = weak.upgrade() else { return };
                let app_state = app.global::<AppState>();
                let st = state.lock().unwrap();
                let idx = app_state.get_selected_index();
                let filtered = filtered_items(&st);
                let Some(item) = filtered.get(idx as usize).map(|i| (*i).clone()) else {
                    return;
                };
                let files = st.scanned_files.clone();
                drop(st);
                let restrict = if scope == 1 {
                    Some(item.aircraft.clone())
                } else {
                    None
                };
                match build_plan(&item, &files, &from_guid, &to_guid, restrict.as_deref()) {
                    Ok(manifest) => {
                        let summary = plan_summary_for_ui(&manifest);
                        *pending.lock().unwrap() = Some(PendingApply { manifest });
                        app_state.set_plan_summary(summary);
                        app_state.set_last_result(SharedString::new());
                        app_state.set_plan_visible(true);
                    }
                    Err(e) => {
                        app_state
                            .set_last_result(SharedString::from(format!("plan failed: {e:#}")));
                        app_state.set_plan_visible(true);
                    }
                }
            },
        );
    }

    // confirm-apply
    {
        let weak = app.as_weak();
        let state = state.clone();
        let pending = pending.clone();
        app_state.on_confirm_apply(move || {
            let Some(app) = weak.upgrade() else { return };
            let Some(pa) = pending.lock().unwrap().take() else {
                return;
            };

            let app_state_now = app.global::<AppState>();
            app_state_now.set_applying(true);

            let weak2 = app.as_weak();
            let state2 = state.clone();
            std::thread::spawn(move || {
                let result = remap::execute(&pa.manifest, true);
                let msg = match result {
                    Ok(done_path) => format!(
                        "Applied. Done marker: {}\nUndo: dcsbinder undo --manifest \"{}\"",
                        done_path.display(),
                        Manifest::path_in(&pa.manifest.backup_dir).display()
                    ),
                    Err(e) => format!("execute failed: {e:#}"),
                };
                let weak3 = weak2.clone();
                let state3 = state2.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    let Some(app) = weak3.upgrade() else { return };
                    let app_state = app.global::<AppState>();
                    app_state.set_applying(false);
                    app_state.set_last_result(SharedString::from(msg));
                    // Auto-rescan after a successful apply so the conflicts list refreshes.
                    trigger_rescan(&app, state3);
                });
            });
        });
    }

    // cancel-apply
    {
        let weak = app.as_weak();
        let pending = pending.clone();
        app_state.on_cancel_apply(move || {
            let Some(app) = weak.upgrade() else { return };
            *pending.lock().unwrap() = None;
            let s = app.global::<AppState>();
            s.set_plan_visible(false);
            s.set_last_result(SharedString::new());
        });
    }

    // filter-changed
    {
        let weak = app.as_weak();
        let state = state.clone();
        app_state.on_filter_changed(move |text: SharedString| {
            let Some(app) = weak.upgrade() else { return };
            let mut st = state.lock().unwrap();
            st.filter = text.to_string().to_lowercase();
            let data = st.clone();
            drop(st);
            populate_conflicts_only(&app, &data);
        });
    }

    // category-changed
    {
        let weak = app.as_weak();
        let state = state.clone();
        app_state.on_category_changed(move |cat: i32| {
            let Some(app) = weak.upgrade() else { return };
            let mut st = state.lock().unwrap();
            st.category = cat;
            let data = st.clone();
            drop(st);
            populate_conflicts_only(&app, &data);
        });
    }

    // request-bulk-plan
    {
        let weak = app.as_weak();
        let state = state.clone();
        let pending = pending.clone();
        app_state.on_request_bulk_plan(move || {
            let Some(app) = weak.upgrade() else { return };
            let app_state = app.global::<AppState>();
            let st = state.lock().unwrap();
            let scope = app_state.get_apply_scope();
            let filtered: Vec<Item> = filtered_items(&st).into_iter().cloned().collect();
            let files = st.scanned_files.clone();
            drop(st);
            match build_bulk_plan(&filtered, &files, scope) {
                Ok(Some(manifest)) => {
                    let summary = plan_summary_for_ui(&manifest);
                    *pending.lock().unwrap() = Some(PendingApply { manifest });
                    app_state.set_plan_summary(summary);
                    app_state.set_last_result(SharedString::new());
                    app_state.set_plan_visible(true);
                }
                Ok(None) => {
                    app_state.set_last_result(SharedString::from(
                        "Nothing to apply — no items in the current filter have a clear live target.",
                    ));
                    app_state.set_plan_visible(true);
                }
                Err(e) => {
                    app_state.set_last_result(SharedString::from(format!(
                        "bulk plan failed: {e:#}"
                    )));
                    app_state.set_plan_visible(true);
                }
            }
        });
    }

    // undo-last
    {
        let weak = app.as_weak();
        let state = state.clone();
        app_state.on_undo_last(move || {
            let Some(app) = weak.upgrade() else { return };
            let app_state = app.global::<AppState>();
            app_state.set_applying(true);
            let weak2 = app.as_weak();
            let state2 = state.clone();
            std::thread::spawn(move || {
                let msg = match undo_last_op() {
                    Ok(Some(path)) => format!("Undone: {}", path.display()),
                    Ok(None) => "No operation to undo.".to_string(),
                    Err(e) => format!("undo failed: {e:#}"),
                };
                let weak3 = weak2.clone();
                let state3 = state2.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    let Some(app) = weak3.upgrade() else { return };
                    let app_state = app.global::<AppState>();
                    app_state.set_applying(false);
                    app_state.set_last_result(SharedString::from(msg));
                    app_state.set_plan_visible(true); // surface the result
                    trigger_rescan(&app, state3);
                });
            });
        });
    }
}

fn trigger_rescan(app: &App, state: Arc<Mutex<AppData>>) {
    let app_state = app.global::<AppState>();
    app_state.set_scanning(true);
    app_state.set_status_text(SharedString::from("Scanning..."));

    let weak = app.as_weak();
    std::thread::spawn(move || {
        let mut new_state = perform_scan();
        // Preserve the existing filter so a rescan after applying a remap
        // doesn't surprise the user by clearing their search.
        {
            let st = state.lock().unwrap();
            new_state.filter.clone_from(&st.filter);
            new_state.category = st.category;
        }
        let weak2 = weak.clone();
        let state2 = state.clone();
        let _ = slint::invoke_from_event_loop(move || {
            let Some(app) = weak2.upgrade() else { return };
            *state2.lock().unwrap() = new_state.clone();
            populate_ui_models(&app, &new_state);
            let app_state = app.global::<AppState>();
            app_state.set_scanning(false);
        });
    });
}

fn perform_scan() -> AppData {
    let installs = config::discover_installs();
    let mut scanned_files: Vec<ScannedFile> = Vec::new();
    for inst in &installs {
        scanned_files.extend(scanner::scan(&inst.input_root));
    }
    let live_devices = device::enumerate().unwrap_or_default();
    let live_pairs: Vec<(String, Guid)> = live_devices
        .iter()
        .map(|d| (d.product_name.clone(), d.instance_guid))
        .collect();

    let conflicts = detect(&scanned_files);
    let orphans = detect_orphans(&scanned_files, &live_pairs);

    let items = build_items(&conflicts, &orphans, &live_pairs);

    AppData {
        installs,
        scanned_files,
        live_devices,
        items,
        filter: String::new(),
        category: 0,
    }
}

fn build_items(conflicts: &[Conflict], orphans: &[Orphan], live: &[(String, Guid)]) -> Vec<Item> {
    let mut items: Vec<Item> = Vec::new();

    for c in conflicts {
        let live_guid = live
            .iter()
            .find(|(name, _)| name == &c.device_name)
            .map(|(_, g)| g.to_dcs_string());
        items.push(Item {
            install_root: c.install_root.clone(),
            aircraft: c.aircraft.clone(),
            subtype: c.subtype,
            device_name: c.device_name.clone(),
            candidates: c.candidates.clone(),
            orphan_target_guid: None,
            live_guid,
        });
    }

    for o in orphans {
        items.push(Item {
            install_root: o.install_root.clone(),
            aircraft: o.aircraft.clone(),
            subtype: o.subtype,
            device_name: o.device_name.clone(),
            candidates: vec![Candidate {
                guid: o.stale_guid.clone(),
                path: o.stale_path.clone(),
            }],
            orphan_target_guid: Some(o.live_guid.clone()),
            live_guid: Some(o.live_guid.clone()),
        });
    }

    items.sort_by(|a, b| {
        a.aircraft
            .cmp(&b.aircraft)
            .then_with(|| a.subtype.as_str().cmp(b.subtype.as_str()))
            .then_with(|| a.device_name.cmp(&b.device_name))
    });
    items
}

fn filtered_items(data: &AppData) -> Vec<&Item> {
    let needle = &data.filter;
    data.items
        .iter()
        .filter(|i| match data.category {
            1 => i.orphan_target_guid.is_none(), // conflicts only
            2 => i.orphan_target_guid.is_some(), // orphans only
            _ => true,
        })
        .filter(|i| {
            needle.is_empty()
                || i.aircraft.to_lowercase().contains(needle)
                || i.device_name.to_lowercase().contains(needle)
                || i.subtype.as_str().contains(needle)
        })
        .collect()
}

fn populate_conflicts_only(app: &App, data: &AppData) {
    let app_state = app.global::<AppState>();
    let rows: Vec<ConflictRow> = filtered_items(data)
        .iter()
        .map(|i| item_to_row(i))
        .collect();
    app_state.set_conflicts(ModelRc::from(Rc::new(VecModel::from(rows))));
    // Reset selection: the indices changed.
    app_state.set_selected_index(-1);
    app_state.set_diff_segments(ModelRc::from(Rc::new(VecModel::from(
        Vec::<DiffSegment>::new(),
    ))));
    app_state.set_sbs_rows(ModelRc::from(Rc::new(VecModel::from(Vec::<SbsRow>::new()))));
}

fn populate_ui_models(app: &App, data: &AppData) {
    let app_state = app.global::<AppState>();

    let install_rows: Vec<InstallInfo> = data
        .installs
        .iter()
        .map(|i| InstallInfo {
            label: SharedString::from(match i.flavor {
                config::DcsFlavor::Stable => "DCS (stable)",
                config::DcsFlavor::OpenBeta => "DCS Open Beta",
                config::DcsFlavor::ServerBeta => "DCS Server Beta",
            }),
            input_root: SharedString::from(i.input_root.display().to_string()),
            aircraft_count: count_aircraft(data, &i.input_root) as i32,
            device_count: count_active_files(data, &i.input_root) as i32,
        })
        .collect();
    app_state.set_installs(ModelRc::from(Rc::new(VecModel::from(install_rows))));

    let conflict_rows: Vec<ConflictRow> = filtered_items(data)
        .iter()
        .map(|i| item_to_row(i))
        .collect();
    app_state.set_conflicts(ModelRc::from(Rc::new(VecModel::from(conflict_rows))));

    // Status bar.
    let conflicts_count = data
        .items
        .iter()
        .filter(|i| i.orphan_target_guid.is_none())
        .count();
    let orphans_count = data
        .items
        .iter()
        .filter(|i| i.orphan_target_guid.is_some())
        .count();
    app_state.set_status_text(SharedString::from(format!(
        "{} install(s) · {} conflict(s) · {} orphan(s) · {} live device(s)",
        data.installs.len(),
        conflicts_count,
        orphans_count,
        data.live_devices.len(),
    )));

    // DCS-running indicator.
    if let Some(pid) = config::dcs_running() {
        app_state.set_dcs_running(true);
        app_state.set_dcs_running_pid(pid as i32);
    } else {
        app_state.set_dcs_running(false);
    }

    // Reset selection.
    app_state.set_selected_index(-1);
    app_state.set_diff_segments(ModelRc::from(Rc::new(VecModel::from(
        Vec::<DiffSegment>::new(),
    ))));
}

fn count_aircraft(data: &AppData, install_root: &std::path::Path) -> usize {
    use std::collections::HashSet;
    data.scanned_files
        .iter()
        .filter(|f| f.install_root == install_root)
        .map(|f| f.aircraft.as_str())
        .collect::<HashSet<_>>()
        .len()
}

fn count_active_files(data: &AppData, install_root: &std::path::Path) -> usize {
    data.scanned_files
        .iter()
        .filter(|f| f.install_root == install_root)
        .filter(|f| matches!(&f.status, FileStatus::Active { .. }))
        .count()
}

fn item_to_row(item: &Item) -> ConflictRow {
    let live_canonical = item.live_guid.as_deref();
    let is_live = |guid_str: &str| -> bool {
        match (live_canonical, Guid::parse_dcs(&format!("{{{guid_str}}}"))) {
            (Some(live), Ok(g)) => live == g.to_dcs_string().as_str(),
            _ => false,
        }
    };

    let (a, b, extra) = if item.candidates.is_empty() {
        ((String::new(), false), (String::new(), false), 0)
    } else if item.candidates.len() == 1 {
        (
            (
                item.candidates[0].guid.clone(),
                is_live(&item.candidates[0].guid),
            ),
            (String::new(), false),
            0,
        )
    } else {
        let a = (
            item.candidates[0].guid.clone(),
            is_live(&item.candidates[0].guid),
        );
        let b = (
            item.candidates[1].guid.clone(),
            is_live(&item.candidates[1].guid),
        );
        let extra = (item.candidates.len() - 2) as i32;
        (a, b, extra)
    };

    ConflictRow {
        aircraft: SharedString::from(item.aircraft.clone()),
        device_name: SharedString::from(item.device_name.clone()),
        subtype: SharedString::from(item.subtype.as_str().to_string()),
        candidate_a_guid: SharedString::from(a.0),
        candidate_a_live: a.1,
        candidate_b_guid: SharedString::from(b.0),
        candidate_b_live: b.1,
        extra_candidates: extra,
        is_orphan: item.orphan_target_guid.is_some(),
        orphan_target_guid: SharedString::from(item.orphan_target_guid.clone().unwrap_or_default()),
    }
}

/// Other devices configured in the same `install / aircraft / subtype` as
/// the selected item. Used by the aircraft-context strip.
fn build_aircraft_context(data: &AppData, item: &Item) -> Vec<AircraftDevice> {
    use std::collections::HashSet;

    // Live device-name set (case-sensitive product names).
    let live_names: HashSet<String> = data
        .live_devices
        .iter()
        .map(|d| d.product_name.clone())
        .collect();

    // Names that have conflicts (same device, multiple GUIDs) — gleaned from the
    // full items list, not just visible filtered items.
    let conflicting_names: HashSet<String> = data
        .items
        .iter()
        .filter(|i| {
            i.install_root == item.install_root
                && i.aircraft == item.aircraft
                && i.subtype == item.subtype
                && i.orphan_target_guid.is_none()
                && i.candidates.len() >= 2
        })
        .map(|i| i.device_name.clone())
        .collect();

    let mut by_name: std::collections::BTreeMap<String, AircraftDevice> =
        std::collections::BTreeMap::new();

    for f in &data.scanned_files {
        if f.install_root != item.install_root
            || f.aircraft != item.aircraft
            || f.subtype != Some(item.subtype)
        {
            continue;
        }
        if let scanner::FileStatus::Active { device_name, guid } = &f.status {
            if device_name == &item.device_name {
                continue; // skip the device the user is currently looking at
            }
            // Prefer LIVE candidates when a device has multiple GUID candidates.
            let is_live = live_names.contains(device_name);
            let has_conflict = conflicting_names.contains(device_name);
            let entry = AircraftDevice {
                device_name: SharedString::from(device_name.clone()),
                guid: SharedString::from(guid.clone()),
                is_live,
                has_conflict,
            };
            by_name
                .entry(device_name.clone())
                .and_modify(|d| {
                    if is_live {
                        *d = entry.clone();
                    }
                })
                .or_insert(entry);
        }
    }

    by_name.into_values().collect()
}

/// The aircraft folders the planner would touch for this item under
/// "all aircraft" scope, sorted alphabetically. The currently-selected
/// aircraft is marked so the UI can highlight it.
fn build_affected_aircraft_list(data: &AppData, item: &Item) -> Vec<AffectedAircraft> {
    use std::collections::BTreeSet;
    let names: BTreeSet<&str> = data
        .scanned_files
        .iter()
        .filter(|f| {
            f.install_root == item.install_root
                && f.subtype == Some(item.subtype)
                && matches!(
                    &f.status,
                    scanner::FileStatus::Active { device_name, .. }
                        if device_name == &item.device_name
                )
        })
        .map(|f| f.aircraft.as_str())
        .collect();
    names
        .into_iter()
        .map(|name| AffectedAircraft {
            name: SharedString::from(name),
            is_selected: name == item.aircraft,
        })
        .collect()
}

fn compute_diffs(item: &Item) -> (Vec<DiffSegment>, Vec<SbsRow>) {
    let (a_bytes, b_bytes) = match item.candidates.as_slice() {
        [] => (Vec::new(), Vec::new()),
        [only] => (std::fs::read(&only.path).unwrap_or_default(), Vec::new()),
        [a, b, ..] => (
            std::fs::read(&a.path).unwrap_or_default(),
            std::fs::read(&b.path).unwrap_or_default(),
        ),
    };
    let a_text = String::from_utf8_lossy(&a_bytes);
    let b_text = String::from_utf8_lossy(&b_bytes);
    let diff = TextDiff::from_lines(&a_text, &b_text);

    let mut inline: Vec<DiffSegment> = Vec::new();
    let mut sbs: Vec<SbsRow> = Vec::new();

    let hunks = diff.grouped_ops(3);
    for ops in &hunks {
        let (a_start, a_end, b_start, b_end) = hunk_range(ops);
        let header_text = format!(
            "@@ -{},{} +{},{} @@",
            a_start + 1,
            a_end - a_start,
            b_start + 1,
            b_end - b_start
        );
        inline.push(DiffSegment {
            text: SharedString::from(header_text.clone()),
            kind: 3,
            line_a: -1,
            line_b: -1,
        });
        sbs.push(SbsRow {
            left_text: SharedString::new(),
            right_text: SharedString::new(),
            left_kind: 0,
            right_kind: 0,
            line_a: -1,
            line_b: -1,
            header_kind: 3,
            header_text: SharedString::from(header_text),
        });

        // Two passes per hunk: one for inline (sequential), one for SbS
        // (delete/insert runs paired).
        let mut pending_del: Vec<(usize, String)> = Vec::new();
        let mut pending_ins: Vec<(usize, String)> = Vec::new();

        for op in ops {
            for change in diff.iter_changes(op) {
                let raw = change
                    .to_string()
                    .trim_end_matches('\n')
                    .replace('\t', "    ");
                let la = change.old_index().map_or(-1, |i| i as i32 + 1);
                let lb = change.new_index().map_or(-1, |i| i as i32 + 1);
                let kind = match change.tag() {
                    ChangeTag::Equal => 0,
                    ChangeTag::Insert => 1,
                    ChangeTag::Delete => 2,
                };
                inline.push(DiffSegment {
                    text: SharedString::from(raw.clone()),
                    kind,
                    line_a: la,
                    line_b: lb,
                });
                match change.tag() {
                    ChangeTag::Equal => {
                        flush_pending_sbs(&mut pending_del, &mut pending_ins, &mut sbs);
                        sbs.push(SbsRow {
                            left_text: SharedString::from(raw.clone()),
                            right_text: SharedString::from(raw),
                            left_kind: 0,
                            right_kind: 0,
                            line_a: la,
                            line_b: lb,
                            header_kind: 0,
                            header_text: SharedString::new(),
                        });
                    }
                    ChangeTag::Delete => {
                        pending_del.push((la as usize, raw));
                    }
                    ChangeTag::Insert => {
                        pending_ins.push((lb as usize, raw));
                    }
                }
            }
        }
        flush_pending_sbs(&mut pending_del, &mut pending_ins, &mut sbs);
    }

    (inline, sbs)
}

fn hunk_range(ops: &[similar::DiffOp]) -> (usize, usize, usize, usize) {
    let a_start = ops
        .first()
        .map(similar::DiffOp::old_range)
        .map_or(0, |r| r.start);
    let a_end = ops
        .last()
        .map(similar::DiffOp::old_range)
        .map_or(0, |r| r.end);
    let b_start = ops
        .first()
        .map(similar::DiffOp::new_range)
        .map_or(0, |r| r.start);
    let b_end = ops
        .last()
        .map(similar::DiffOp::new_range)
        .map_or(0, |r| r.end);
    (a_start, a_end, b_start, b_end)
}

fn flush_pending_sbs(
    dels: &mut Vec<(usize, String)>,
    ins: &mut Vec<(usize, String)>,
    sbs: &mut Vec<SbsRow>,
) {
    let max = dels.len().max(ins.len());
    for i in 0..max {
        let (la, left) = dels
            .get(i)
            .map_or((-1_i32, String::new()), |(n, s)| (*n as i32, s.clone()));
        let (lb, right) = ins
            .get(i)
            .map_or((-1_i32, String::new()), |(n, s)| (*n as i32, s.clone()));
        let left_present = dels.get(i).is_some();
        let right_present = ins.get(i).is_some();
        sbs.push(SbsRow {
            left_text: SharedString::from(left),
            right_text: SharedString::from(right),
            left_kind: if left_present { 1 } else { 2 },
            right_kind: if right_present { 1 } else { 2 },
            line_a: la,
            line_b: lb,
            header_kind: 0,
            header_text: SharedString::new(),
        });
    }
    dels.clear();
    ins.clear();
}

/// Build a single combined manifest covering every item in `items` for which
/// a "live" target can be auto-determined. Each item contributes its own
/// backups + mutations to one big plan.
///
/// Auto-resolution rules per item:
///   - Orphan: source = the stale GUID, target = the live GUID.
///   - Conflict with one live + one stale: source = stale, target = live.
///   - Conflict with no live candidate: skipped (user must resolve manually).
///   - Conflict with multiple live: skipped (ambiguous).
fn build_bulk_plan(items: &[Item], files: &[ScannedFile], scope: i32) -> Result<Option<Manifest>> {
    use dcsbinder_core::remap::{BackupEntry, Manifest, Mutation, OperationKind, MANIFEST_VERSION};
    use std::collections::BTreeSet;
    use uuid::Uuid;

    let backup_root = config::app_data_dir()
        .ok_or_else(|| anyhow::anyhow!("could not resolve %APPDATA%/DCSBinder"))?
        .join("backups");
    std::fs::create_dir_all(&backup_root)?;

    let op_id = Uuid::now_v7().to_string();
    let timestamp = time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default();
    let safe_ts = timestamp.replace([':', '.'], "-");
    let short: String = op_id
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .take(8)
        .collect();
    let bulk_backup_dir = backup_root.join(format!("{safe_ts}_bulk_{short}"));

    let mut backups: Vec<BackupEntry> = Vec::new();
    let mut mutations: Vec<Mutation> = Vec::new();
    let mut seen_backup_src: BTreeSet<std::path::PathBuf> = BTreeSet::new();
    let mut device_summary: Vec<String> = Vec::new();
    let mut install_root_for_manifest: Option<std::path::PathBuf> = None;
    let mut subtype_for_manifest: Option<String> = None;

    for item in items {
        let (source_str, target_str) = if let Some(target) = &item.orphan_target_guid {
            (Some(item.candidates[0].guid.clone()), Some(target.clone()))
        } else if item.candidates.len() == 2 {
            // Pick the live candidate as target, stale as source.
            let live_guid = item.live_guid.as_deref();
            let a = &item.candidates[0];
            let b = &item.candidates[1];
            let parse = |g: &str| Guid::parse_dcs(&format!("{{{g}}}"));
            match (parse(&a.guid), parse(&b.guid), live_guid) {
                (Ok(pa), Ok(pb), Some(live)) => {
                    let a_live = pa.to_dcs_string() == live;
                    let b_live = pb.to_dcs_string() == live;
                    if a_live && !b_live {
                        (Some(b.guid.clone()), Some(a.guid.clone()))
                    } else if b_live && !a_live {
                        (Some(a.guid.clone()), Some(b.guid.clone()))
                    } else {
                        (None, None) // skip — ambiguous or both stale
                    }
                }
                _ => (None, None),
            }
        } else {
            (None, None)
        };

        let (Some(source_str), Some(target_str)) = (source_str, target_str) else {
            continue;
        };
        let restrict_aircraft = (scope == 1).then(|| item.aircraft.clone());
        let Ok(m) = build_plan(
            item,
            files,
            &source_str,
            &target_str,
            restrict_aircraft.as_deref(),
        ) else {
            continue;
        };

        install_root_for_manifest = Some(m.install_root.clone());
        subtype_for_manifest = Some(m.subtype.clone());
        for b in m.backups {
            if seen_backup_src.insert(b.src.clone()) {
                // rewrite backup path under the bulk backup dir.
                let rewrote = rewrite_backup_path(&b.backup, &m.backup_dir, &bulk_backup_dir);
                backups.push(BackupEntry {
                    src: b.src,
                    backup: rewrote,
                    blake3: b.blake3,
                    size: b.size,
                });
            }
        }
        for mu in m.mutations {
            mutations.push(rewrite_mutation(&mu, &m.backup_dir, &bulk_backup_dir));
        }
        device_summary.push(format!(
            "{} ({}{})",
            item.device_name,
            item.aircraft,
            if scope == 1 { "" } else { ", …" }
        ));
    }

    if mutations.is_empty() {
        return Ok(None);
    }

    let install_root = install_root_for_manifest.unwrap_or_else(|| backup_root.clone());
    let subtype = subtype_for_manifest.unwrap_or_else(|| "joystick".to_string());
    let manifest = Manifest {
        version: MANIFEST_VERSION,
        operation_id: op_id,
        operation: OperationKind::Remap,
        timestamp,
        backup_dir: bulk_backup_dir,
        install_root,
        device_name: format!("BULK: {} item(s)", device_summary.len()),
        subtype,
        source_guid: "<bulk>".to_string(),
        target_guid: "<bulk>".to_string(),
        backups,
        mutations,
    };
    Ok(Some(manifest))
}

fn rewrite_backup_path(
    original: &std::path::Path,
    from_root: &std::path::Path,
    to_root: &std::path::Path,
) -> std::path::PathBuf {
    original
        .strip_prefix(from_root)
        .map_or_else(|_| original.to_path_buf(), |rel| to_root.join(rel))
}

fn rewrite_mutation(
    m: &dcsbinder_core::remap::Mutation,
    from_root: &std::path::Path,
    to_root: &std::path::Path,
) -> dcsbinder_core::remap::Mutation {
    use dcsbinder_core::remap::Mutation as M;
    match m {
        M::WriteFile {
            dst,
            source,
            source_blake3,
        } => M::WriteFile {
            dst: dst.clone(),
            source: source.clone(),
            source_blake3: source_blake3.clone(),
        },
        M::MoveFile { src, dst } => M::MoveFile {
            src: src.clone(),
            dst: rewrite_backup_path(dst, from_root, to_root),
        },
        M::StringReplace {
            path,
            find,
            replace,
            expected_replacements,
        } => M::StringReplace {
            path: path.clone(),
            find: find.clone(),
            replace: replace.clone(),
            expected_replacements: *expected_replacements,
        },
    }
}

fn build_plan(
    item: &Item,
    files: &[ScannedFile],
    from_guid_str: &str,
    to_guid_str: &str,
    restrict_to_aircraft: Option<&str>,
) -> Result<Manifest> {
    let from_guid = Guid::parse_dcs(from_guid_str)
        .with_context(|| format!("invalid from-guid {from_guid_str}"))?;
    let to_guid =
        Guid::parse_dcs(to_guid_str).with_context(|| format!("invalid to-guid {to_guid_str}"))?;
    let backup_root = config::app_data_dir()
        .ok_or_else(|| anyhow::anyhow!("could not resolve %APPDATA%/DCSBinder"))?
        .join("backups");
    std::fs::create_dir_all(&backup_root)?;
    Ok(remap::plan_with_scope(
        &item.install_root,
        &item.device_name,
        item.subtype,
        &from_guid,
        &to_guid,
        files,
        &backup_root,
        restrict_to_aircraft,
    )?)
}

fn plan_summary_for_ui(manifest: &Manifest) -> PlanSummary {
    let mut lines: Vec<SharedString> = Vec::new();
    for (i, m) in manifest.mutations.iter().enumerate() {
        let s = match m {
            Mutation::WriteFile { dst, source, .. } => format!(
                "{:>3}. WRITE   {}  <-  {}",
                i + 1,
                dst.display(),
                source.display()
            ),
            Mutation::MoveFile { src, dst } => format!(
                "{:>3}. MOVE    {}  ->  {}",
                i + 1,
                src.display(),
                dst.display()
            ),
            Mutation::StringReplace {
                path,
                expected_replacements,
                find,
                replace,
            } => format!(
                "{:>3}. REWRITE {}  ({expected_replacements}x  {find}  ->  {replace})",
                i + 1,
                path.display()
            ),
        };
        lines.push(SharedString::from(s));
    }
    let aircraft_count = manifest
        .mutations
        .iter()
        .filter_map(|m| match m {
            Mutation::WriteFile { dst, .. } => Some(dst.clone()),
            _ => None,
        })
        .filter_map(|p| {
            p.parent()
                .and_then(|p| p.parent())
                .map(std::path::Path::to_path_buf)
        })
        .collect::<std::collections::HashSet<_>>()
        .len() as i32;

    PlanSummary {
        device_name: SharedString::from(manifest.device_name.clone()),
        from_guid: SharedString::from(manifest.source_guid.clone()),
        to_guid: SharedString::from(manifest.target_guid.clone()),
        aircraft_count,
        mutation_count: manifest.mutations.len() as i32,
        backup_dir: SharedString::from(manifest.backup_dir.display().to_string()),
        lines: ModelRc::from(Rc::new(VecModel::from(lines))),
    }
}

fn undo_last_op() -> Result<Option<PathBuf>> {
    let backup_root = config::app_data_dir()
        .ok_or_else(|| anyhow::anyhow!("could not resolve %APPDATA%/DCSBinder"))?
        .join("backups");
    if !backup_root.is_dir() {
        return Ok(None);
    }
    let mut completed: Vec<PathBuf> = std::fs::read_dir(&backup_root)?
        .filter_map(std::result::Result::ok)
        .filter_map(|e| {
            let p = e.path();
            if !p.is_dir() {
                return None;
            }
            let m = Manifest::path_in(&p);
            let d = Manifest::done_marker_in(&p);
            (m.is_file() && d.exists()).then_some(m)
        })
        .collect();
    completed.sort_by_key(|p| p.parent().map(std::path::Path::to_path_buf));
    let Some(manifest) = completed.pop() else {
        return Ok(None);
    };
    remap::undo(&manifest)?;
    Ok(Some(manifest))
}
