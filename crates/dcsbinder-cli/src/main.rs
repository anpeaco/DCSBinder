use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use dcsbinder_core::{config, conflict, device, remap, scanner};

#[derive(Parser, Debug)]
#[command(
    name = "dcsbinder",
    version,
    about = "Detect, diff, and remap `DCS World` controller bindings across GUID changes."
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Walk a DCS Input directory and report devices, classifications, and GUID conflicts.
    Scan {
        /// Path to a DCS install's `Config/Input` folder. If omitted, every install
        /// found under `%USERPROFILE%\Saved Games\DCS*\Config\Input` is scanned.
        input_root: Option<PathBuf>,

        /// Show all scanned files, not just conflicts.
        #[arg(long)]
        verbose: bool,
    },
    /// List currently-connected controllers and their `DirectInput` GUIDs.
    Devices,
    /// Remap a chosen binding's content under a new GUID across every aircraft folder.
    Remap {
        /// Device name as it appears in DCS bind filenames (e.g. `MFDLeft`).
        #[arg(long)]
        device: String,
        /// Input subtype directory the device lives in.
        #[arg(long, default_value = "joystick")]
        subtype: SubtypeArg,
        /// GUID whose file contents are the source of truth.
        #[arg(long, value_name = "GUID")]
        from_guid: String,
        /// GUID to write the content under (typically the live one).
        #[arg(long, value_name = "GUID")]
        to_guid: String,
        /// Path to a DCS install's `Config/Input`. If omitted, auto-discovery is used.
        #[arg(long)]
        input_root: Option<PathBuf>,
        /// Print the plan and stop — no files mutated.
        #[arg(long)]
        dry_run: bool,
        /// Skip the interactive confirmation prompt.
        #[arg(long)]
        yes: bool,
    },
    /// Roll back a previous remap operation from its backup manifest.
    Undo {
        /// Roll back the most-recent operation under `%APPDATA%/DCSBinder/backups/`.
        #[arg(long, conflicts_with = "manifest")]
        last: bool,
        /// Path to a `manifest.json` to roll back.
        #[arg(long, value_name = "PATH")]
        manifest: Option<PathBuf>,
    },
    /// (M5) Show the audit log of past remap operations.
    History,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.cmd {
        Cmd::Scan {
            input_root,
            verbose,
        } => cmd_scan(input_root.as_deref(), verbose),
        Cmd::Devices => cmd_devices(),
        Cmd::Remap {
            device,
            subtype,
            from_guid,
            to_guid,
            input_root,
            dry_run,
            yes,
        } => cmd_remap(
            &device,
            subtype.into(),
            &from_guid,
            &to_guid,
            input_root.as_deref(),
            dry_run,
            yes,
        ),
        Cmd::Undo { last, manifest } => cmd_undo(last, manifest.as_deref()),
        Cmd::History => {
            cmd_stub("history", "M5");
            Ok(())
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn cmd_scan(input_root: Option<&std::path::Path>, verbose: bool) -> Result<()> {
    let roots = resolve_roots(input_root)?;
    if roots.is_empty() {
        anyhow::bail!(
            "no DCS install found. Provide an explicit `<input-root>` or install DCS \
             under `%USERPROFILE%\\Saved Games\\`."
        );
    }

    if let Some(pid) = config::dcs_running() {
        eprintln!(
            "WARNING: DCS.exe is currently running (PID {pid}). Close DCS before any remap; \
             scanning is safe but file mutations will be refused later."
        );
        eprintln!();
    }

    let live = match device::enumerate() {
        Ok(devices) => devices,
        Err(e) => {
            eprintln!("warning: could not enumerate live DirectInput devices: {e}");
            eprintln!();
            Vec::new()
        }
    };
    let live_guid_set: HashSet<String> = live
        .iter()
        .map(|d| d.instance_guid.to_dcs_string())
        .collect();

    for root in &roots {
        let files = scanner::scan(root);
        let conflicts = conflict::detect(&files);

        println!("=== {} ===", root.display());
        println!();

        if verbose {
            print_file_listing(&files, &live_guid_set);
            println!();
        }

        print_conflict_report(&conflicts, &live_guid_set);
        println!();
    }

    Ok(())
}

fn resolve_roots(input_root: Option<&std::path::Path>) -> Result<Vec<PathBuf>> {
    if let Some(root) = input_root {
        let canon = std::fs::canonicalize(root)
            .with_context(|| format!("could not canonicalize {}", root.display()))?;
        if !canon.is_dir() {
            anyhow::bail!("{} is not a directory", canon.display());
        }
        return Ok(vec![canon]);
    }
    let installs = config::discover_installs();
    Ok(installs.into_iter().map(|i| i.input_root).collect())
}

fn print_file_listing(files: &[scanner::ScannedFile], live: &HashSet<String>) {
    use scanner::FileStatus;

    let mut active = 0usize;
    let mut archived = 0usize;
    let mut modifiers = 0usize;
    let mut exported = 0usize;
    let mut malformed = 0usize;

    for f in files {
        match &f.status {
            FileStatus::Active { device_name, guid } => {
                active += 1;
                let live_marker = liveness_marker(guid, live);
                println!(
                    "  ACTIVE   {live_marker} {} / {} / {device_name} {{{guid}}}",
                    f.aircraft,
                    f.subtype.map_or("-", scanner::Subtype::as_str),
                );
            }
            FileStatus::UserArchived {
                device_name,
                guid,
                suffix,
            } => {
                archived += 1;
                println!(
                    "  ARCHIVED      {} / {} / {device_name} {{{guid}}}{suffix}",
                    f.aircraft,
                    f.subtype.map_or("-", scanner::Subtype::as_str),
                );
            }
            FileStatus::Modifiers => {
                modifiers += 1;
                println!("  MODIFIERS     {} / modifiers.lua", f.aircraft);
            }
            FileStatus::ExportedProfile => {
                exported += 1;
                println!(
                    "  PROFILE       {} / {} / {}",
                    f.aircraft,
                    f.subtype.map_or("-", scanner::Subtype::as_str),
                    f.path.file_name().and_then(|n| n.to_str()).unwrap_or("?"),
                );
            }
            FileStatus::Malformed { reason } => {
                malformed += 1;
                println!(
                    "  MALFORMED     {} / {} / {} ({reason})",
                    f.aircraft,
                    f.subtype.map_or("-", scanner::Subtype::as_str),
                    f.path.file_name().and_then(|n| n.to_str()).unwrap_or("?"),
                );
            }
        }
    }
    println!();
    println!(
        "Files: {active} active, {archived} archived, {modifiers} modifiers, {exported} profiles, {malformed} malformed (total {})",
        files.len(),
    );
}

fn print_conflict_report(conflicts: &[conflict::Conflict], live: &HashSet<String>) {
    if conflicts.is_empty() {
        println!("No GUID conflicts detected.");
        return;
    }

    println!("Detected {} GUID conflict(s):", conflicts.len());
    println!();
    for c in conflicts {
        println!(
            "  [{}] {} / {}",
            c.subtype.as_str(),
            c.aircraft,
            c.device_name,
        );
        for cand in &c.candidates {
            let marker = liveness_marker(&cand.guid, live);
            println!("      {marker} {{{}}}", cand.guid);
            println!("            {}", cand.path.display());
        }
        println!();
    }
}

fn liveness_marker(guid: &str, live: &HashSet<String>) -> &'static str {
    // Case-insensitive comparison: parse both sides into the canonical form.
    let Ok(parsed) = device::guid::Guid::parse_dcs(&format!("{{{guid}}}")) else {
        return "[?    ]";
    };
    let canonical = parsed.to_dcs_string();
    if live.contains(&canonical) {
        "[LIVE ]"
    } else if live.is_empty() {
        "[?    ]"
    } else {
        "[STALE]"
    }
}

#[derive(clap::ValueEnum, Clone, Debug)]
enum SubtypeArg {
    Joystick,
    Keyboard,
    Mouse,
    Trackir,
}

impl From<SubtypeArg> for scanner::Subtype {
    fn from(s: SubtypeArg) -> Self {
        match s {
            SubtypeArg::Joystick => Self::Joystick,
            SubtypeArg::Keyboard => Self::Keyboard,
            SubtypeArg::Mouse => Self::Mouse,
            SubtypeArg::Trackir => Self::TrackIr,
        }
    }
}

fn cmd_remap(
    device_name: &str,
    subtype: scanner::Subtype,
    from_guid: &str,
    to_guid: &str,
    input_root: Option<&std::path::Path>,
    dry_run: bool,
    yes: bool,
) -> Result<()> {
    let roots = resolve_roots(input_root)?;
    if roots.len() != 1 {
        anyhow::bail!(
            "remap requires exactly one --input-root (found {}). Pass --input-root \
             explicitly when more than one DCS install is present.",
            roots.len()
        );
    }
    let install_root = &roots[0];

    if let Some(pid) = config::dcs_running() {
        anyhow::bail!(
            "DCS.exe is running (PID {pid}). Close DCS before remap (sharing-violation risk)."
        );
    }

    let source = device::guid::Guid::parse_dcs(from_guid)
        .with_context(|| format!("invalid --from-guid `{from_guid}`"))?;
    let target = device::guid::Guid::parse_dcs(to_guid)
        .with_context(|| format!("invalid --to-guid `{to_guid}`"))?;

    let files = scanner::scan(install_root);

    let backup_root = config::app_data_dir()
        .ok_or_else(|| anyhow::anyhow!("could not resolve %APPDATA%/DCSBinder"))?
        .join("backups");
    std::fs::create_dir_all(&backup_root)
        .with_context(|| format!("creating backup root {}", backup_root.display()))?;

    let manifest = remap::plan(
        install_root,
        device_name,
        subtype,
        &source,
        &target,
        &files,
        &backup_root,
    )?;

    println!("Plan for remap:");
    println!("  device        : {}", manifest.device_name);
    println!("  subtype       : {}", manifest.subtype);
    println!("  from-guid     : {}", manifest.source_guid);
    println!("  to-guid       : {}", manifest.target_guid);
    println!(
        "  backups       : {} file(s) to snapshot",
        manifest.backups.len()
    );
    println!("  mutations     : {} step(s)", manifest.mutations.len());
    println!("  backup-dir    : {}", manifest.backup_dir.display());
    println!();
    for (i, m) in manifest.mutations.iter().enumerate() {
        match m {
            remap::Mutation::WriteFile { dst, source, .. } => {
                println!(
                    "    {:>3}. WRITE   {}  <-  {}",
                    i + 1,
                    dst.display(),
                    source.display()
                );
            }
            remap::Mutation::MoveFile { src, dst } => {
                println!(
                    "    {:>3}. MOVE    {}  ->  {}",
                    i + 1,
                    src.display(),
                    dst.display()
                );
            }
            remap::Mutation::StringReplace {
                path,
                find,
                replace,
                expected_replacements,
            } => {
                println!(
                    "    {:>3}. REWRITE {} ({expected_replacements}x  {find}  ->  {replace})",
                    i + 1,
                    path.display(),
                );
            }
        }
    }
    println!();

    if dry_run {
        println!("--dry-run: stopping before any file mutation.");
        return Ok(());
    }

    if !yes && !prompt_confirm("Proceed?") {
        println!("Aborted.");
        return Ok(());
    }

    let done = remap::execute(&manifest, true)?;
    println!();
    println!("Done. Finalize marker written at:");
    println!("    {}", done.display());
    println!(
        "To undo: dcsbinder undo --manifest \"{}\"",
        remap::Manifest::path_in(&manifest.backup_dir).display()
    );
    Ok(())
}

fn cmd_undo(last: bool, manifest: Option<&std::path::Path>) -> Result<()> {
    let manifest_path: PathBuf = if let Some(p) = manifest {
        p.to_path_buf()
    } else if last {
        let backup_root = config::app_data_dir()
            .ok_or_else(|| anyhow::anyhow!("could not resolve %APPDATA%/DCSBinder"))?
            .join("backups");
        let mut completed: Vec<PathBuf> = std::fs::read_dir(&backup_root)
            .with_context(|| format!("reading {}", backup_root.display()))?
            .filter_map(Result::ok)
            .filter_map(|e| {
                let path = e.path();
                if path.is_dir() {
                    let m = remap::Manifest::path_in(&path);
                    let d = remap::Manifest::done_marker_in(&path);
                    (m.is_file() && d.exists()).then_some(m)
                } else {
                    None
                }
            })
            .collect();
        // operation_id is uuidv7 (sortable). For "last" we sort by parent
        // directory name (timestamp-prefixed) and pick the latest.
        completed.sort_by_key(|p| p.parent().map(Path::to_path_buf));
        completed.pop().ok_or_else(|| {
            anyhow::anyhow!(
                "no completed operations found under {}",
                backup_root.display()
            )
        })?
    } else {
        anyhow::bail!("pass --last or --manifest <PATH>");
    };

    println!("Undoing {}...", manifest_path.display());
    remap::undo(&manifest_path)?;
    println!("Done.");
    Ok(())
}

fn prompt_confirm(msg: &str) -> bool {
    use std::io::Write as _;
    print!("{msg} [y/N]: ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    if std::io::stdin().read_line(&mut line).is_err() {
        return false;
    }
    matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

fn cmd_devices() -> Result<()> {
    let devices = device::enumerate().context("enumerating DirectInput devices")?;
    if devices.is_empty() {
        println!("No game controllers currently attached.");
        return Ok(());
    }
    println!("{} game controller(s) attached:", devices.len());
    println!();
    for d in &devices {
        println!("  {}", d.product_name);
        println!("      instance: {}", d.instance_guid);
        println!("      product : {}", d.product_guid);
        println!();
    }
    Ok(())
}

fn cmd_stub(name: &str, milestone: &str) {
    eprintln!("`{name}` lands in {milestone}. Not yet implemented.");
}
