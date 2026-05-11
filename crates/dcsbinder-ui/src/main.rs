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
            let filtered = filtered_items(&st);
            let (segments, sbs) = if let Some(item) = filtered.get(idx as usize) {
                compute_diffs(item)
            } else {
                (Vec::new(), Vec::new())
            };
            drop(st);
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
        app_state.on_request_plan(move |from_guid: SharedString, to_guid: SharedString| {
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
            match build_plan(&item, &files, &from_guid, &to_guid) {
                Ok(manifest) => {
                    let summary = plan_summary_for_ui(&manifest);
                    *pending.lock().unwrap() = Some(PendingApply { manifest });
                    app_state.set_plan_summary(summary);
                    app_state.set_last_result(SharedString::new());
                    app_state.set_plan_visible(true);
                }
                Err(e) => {
                    app_state.set_last_result(SharedString::from(format!("plan failed: {e:#}")));
                    app_state.set_plan_visible(true);
                }
            }
        });
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
        new_state.filter.clone_from(&state.lock().unwrap().filter);
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
    if data.filter.is_empty() {
        return data.items.iter().collect();
    }
    let needle = &data.filter;
    data.items
        .iter()
        .filter(|i| {
            i.aircraft.to_lowercase().contains(needle)
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

fn flush_pending(dels: &mut Vec<String>, ins: &mut Vec<String>, sbs: &mut Vec<SbsRow>) {
    let max = dels.len().max(ins.len());
    for i in 0..max {
        let left = dels.get(i).cloned();
        let right = ins.get(i).cloned();
        sbs.push(SbsRow {
            left_text: SharedString::from(left.clone().unwrap_or_default()),
            right_text: SharedString::from(right.clone().unwrap_or_default()),
            left_kind: if left.is_some() { 1 } else { 2 },
            right_kind: if right.is_some() { 1 } else { 2 },
        });
    }
    dels.clear();
    ins.clear();
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
    let mut pending_del: Vec<String> = Vec::new();
    let mut pending_ins: Vec<String> = Vec::new();

    for change in diff.iter_all_changes() {
        let text = change
            .to_string()
            .trim_end_matches('\n')
            .replace('\t', "    ");
        match change.tag() {
            ChangeTag::Equal => {
                flush_pending(&mut pending_del, &mut pending_ins, &mut sbs);
                inline.push(DiffSegment {
                    text: SharedString::from(text.clone()),
                    kind: 0,
                });
                sbs.push(SbsRow {
                    left_text: SharedString::from(text.clone()),
                    right_text: SharedString::from(text),
                    left_kind: 0,
                    right_kind: 0,
                });
            }
            ChangeTag::Insert => {
                inline.push(DiffSegment {
                    text: SharedString::from(text.clone()),
                    kind: 1,
                });
                pending_ins.push(text);
            }
            ChangeTag::Delete => {
                inline.push(DiffSegment {
                    text: SharedString::from(text.clone()),
                    kind: 2,
                });
                pending_del.push(text);
            }
        }
    }
    flush_pending(&mut pending_del, &mut pending_ins, &mut sbs);

    (inline, sbs)
}

fn build_plan(
    item: &Item,
    files: &[ScannedFile],
    from_guid_str: &str,
    to_guid_str: &str,
) -> Result<Manifest> {
    let from_guid = Guid::parse_dcs(from_guid_str)
        .with_context(|| format!("invalid from-guid {from_guid_str}"))?;
    let to_guid =
        Guid::parse_dcs(to_guid_str).with_context(|| format!("invalid to-guid {to_guid_str}"))?;
    let backup_root = config::app_data_dir()
        .ok_or_else(|| anyhow::anyhow!("could not resolve %APPDATA%/DCSBinder"))?
        .join("backups");
    std::fs::create_dir_all(&backup_root)?;
    Ok(remap::plan(
        &item.install_root,
        &item.device_name,
        item.subtype,
        &from_guid,
        &to_guid,
        files,
        &backup_root,
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
