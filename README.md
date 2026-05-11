# DCSBinder

[![CI](https://github.com/anpeaco/DCSBinder/actions/workflows/ci.yml/badge.svg)](https://github.com/anpeaco/DCSBinder/actions/workflows/ci.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)
[![Release](https://img.shields.io/github/v/release/anpeaco/DCSBinder?include_prereleases&sort=semver)](https://github.com/anpeaco/DCSBinder/releases)

**Status:** pre-alpha (M0 — scaffold). Not yet usable.

Windows reassigns controller GUIDs after USB hub changes, driver reinstalls, or motherboard swaps, and DCS World keys every binding by GUID — so the same physical joystick gets treated as a brand-new device with zero bindings. DCSBinder scans your DCS Input tree, surfaces every GUID conflict, identifies which GUID is currently live, lets you diff the old and new bindings side-by-side, then atomically remaps the chosen content under the live GUID across **every** aircraft folder — backing up everything first.

> _Screenshot placeholder — UI lands in M4._

## Features (v1 scope)

- Scan the entire DCS Input tree (joystick, keyboard, mouse, trackir) across stable, openbeta, and serverbeta installs.
- Detect duplicate-device-name + different-GUID conflicts.
- Identify the currently-live GUID for each connected controller via SDL2.
- Side-by-side diff of conflicting bind files.
- Bulk remap across every aircraft folder in one transaction.
- Full crash-safe backup before any mutation; one-click undo from history.
- Tamper-evident audit log of every change (`history.jsonl` with blake3 hashes).

## Prerequisites

- Windows 11 (Windows 10 likely fine but untested).
- DCS World installed (stable, openbeta, or serverbeta).
- For end users: nothing else — the released MSI bundles everything.
- For developers: see [Build from source](#build-from-source).

## Install (end users)

Download the latest `.msi` from the [Releases page](https://github.com/anpeaco/DCSBinder/releases) and run it. (Releases land at M6.)

## Build from source

```pwsh
# One-time toolchain setup:
winget install --id Rustlang.Rustup -e
winget install --id Microsoft.VisualStudio.2022.BuildTools -e --override "--add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"

# Clone and build:
git clone https://github.com/anpeaco/DCSBinder.git
cd DCSBinder
cargo run -p dcsbinder-cli -- scan        # headless scan (M1+)
cargo run -p dcsbinder-ui                 # Slint UI (M4+)
```

## Safety

- **Always close DCS World before remapping.** DCSBinder refuses to mutate files while `DCS.exe` is running, but a clean exit is still recommended.
- **Backups live in** `%APPDATA%\DCSBinder\backups\<timestamp>\` and are never deleted automatically. You can configure retention in Settings.
- **History log** is appended to `%APPDATA%\DCSBinder\history.jsonl`. Every operation records before/after blake3 hashes for the files it touched.
- **Dry-run preview** shows exactly which files will be written or moved before any mutation. You always confirm.
- If a remap is interrupted (crash, power loss), the next startup detects the un-finalized manifest and offers to roll back.

## Documentation

- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) — module map, two-phase commit, error taxonomy.
- [`docs/FILE_FORMAT.md`](docs/FILE_FORMAT.md) — annotated `.diff.lua` reference and GUID format.
- [`docs/ROADMAP.md`](docs/ROADMAP.md) — milestones M0–M6 and deferred features.
- [`docs/DECISIONS/`](docs/DECISIONS/) — Architecture Decision Records.

## Report a bug / request a feature

[Open an issue on GitHub.](https://github.com/anpeaco/DCSBinder/issues) Please include your DCS version, OS version, and (if relevant) the `%APPDATA%\DCSBinder\history.jsonl` entry for the operation that failed.

## Contributing

PRs welcome once M1 ships. Until then, the design is evolving fast — open an issue to discuss before sending code. CI requires `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and a green `cargo test --workspace`.

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE) at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual-licensed as above, without any additional terms or conditions.

## Disclaimer

DCSBinder is an unofficial third-party tool. It is not affiliated with or endorsed by Eagle Dynamics. "DCS World" is a trademark of Eagle Dynamics.
