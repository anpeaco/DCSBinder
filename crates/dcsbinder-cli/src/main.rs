use std::collections::HashSet;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use dcsbinder_core::{config, conflict, device, scanner};

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
    /// (M3) Remap a chosen binding's content under a new GUID across every aircraft folder.
    Remap,
    /// (M5) Show the audit log of past remap operations.
    History,
    /// (M5) Undo a past remap operation.
    Undo,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.cmd {
        Cmd::Scan {
            input_root,
            verbose,
        } => cmd_scan(input_root.as_deref(), verbose),
        Cmd::Devices => cmd_devices(),
        Cmd::Remap => {
            cmd_stub("remap", "M3");
            Ok(())
        }
        Cmd::History => {
            cmd_stub("history", "M5");
            Ok(())
        }
        Cmd::Undo => {
            cmd_stub("undo", "M5");
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
