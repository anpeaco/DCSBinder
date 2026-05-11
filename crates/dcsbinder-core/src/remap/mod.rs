//! Safety-critical two-phase commit engine for remapping a chosen binding's
//! content under a new GUID across every aircraft folder.
//!
//! Algorithm summary (full detail in `docs/ARCHITECTURE.md`):
//! 1. Plan (no writes).
//! 2. Manifest written via `tempfile::NamedTempFile::persist()`. Existence = started.
//! 3. Backup (full snapshot of affected folder, blake3-hashed).
//! 4. Write (`tmp → fs::rename`, atomic within NTFS directory).
//! 5. Move-not-delete (stale-GUID files → backup folder).
//! 6. Finalize (`manifest.json.done` sibling marker).
//!
//! On startup, an un-`.done` manifest triggers the rollback prompt.
//!
//! Until M3 lands, this module is a stub.
