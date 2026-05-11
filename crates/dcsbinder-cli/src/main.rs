use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use dcsbinder_core::{conflict, scanner};

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
        /// Path to a DCS install's `Config/Input` folder, e.g.
        /// `C:\Users\<you>\Saved Games\DCS.openbeta\Config\Input`.
        input_root: PathBuf,

        /// Show all scanned files, not just conflicts.
        #[arg(long)]
        verbose: bool,
    },
    /// (M2) List currently-connected controllers and their `DirectInput` GUIDs.
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
        } => cmd_scan(&input_root, verbose),
        Cmd::Devices => {
            cmd_stub("devices", "M2");
            Ok(())
        }
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

fn cmd_scan(input_root: &std::path::Path, verbose: bool) -> Result<()> {
    let canon = std::fs::canonicalize(input_root)
        .with_context(|| format!("could not canonicalize {}", input_root.display()))?;
    if !canon.is_dir() {
        anyhow::bail!("{} is not a directory", canon.display());
    }

    let files = scanner::scan(&canon);
    let conflicts = conflict::detect(&files);

    println!("Scanned {}", canon.display());
    println!();

    if verbose {
        print_file_listing(&files);
        println!();
    }

    print_conflict_report(&conflicts);
    Ok(())
}

fn print_file_listing(files: &[scanner::ScannedFile]) {
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
                println!(
                    "  ACTIVE       {} / {} / {device_name} {{{guid}}}",
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
                    "  ARCHIVED     {} / {} / {device_name} {{{guid}}}{suffix}",
                    f.aircraft,
                    f.subtype.map_or("-", scanner::Subtype::as_str),
                );
            }
            FileStatus::Modifiers => {
                modifiers += 1;
                println!("  MODIFIERS    {} / modifiers.lua", f.aircraft);
            }
            FileStatus::ExportedProfile => {
                exported += 1;
                println!(
                    "  PROFILE      {} / {} / {}",
                    f.aircraft,
                    f.subtype.map_or("-", scanner::Subtype::as_str),
                    f.path.file_name().and_then(|n| n.to_str()).unwrap_or("?"),
                );
            }
            FileStatus::Malformed { reason } => {
                malformed += 1;
                println!(
                    "  MALFORMED    {} / {} / {} ({reason})",
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

fn print_conflict_report(conflicts: &[conflict::Conflict]) {
    if conflicts.is_empty() {
        println!("No GUID conflicts detected.");
        return;
    }

    println!("Detected {} GUID conflict(s):", conflicts.len());
    println!();
    for c in conflicts {
        println!(
            "  [{}] {} / {} / v",
            c.subtype.as_str(),
            c.aircraft,
            c.device_name,
        );
        for cand in &c.candidates {
            println!("      {{{}}}", cand.guid);
            println!("          {}", cand.path.display());
        }
        println!();
    }
}

fn cmd_stub(name: &str, milestone: &str) {
    eprintln!("`{name}` lands in {milestone}. Not yet implemented.");
}
