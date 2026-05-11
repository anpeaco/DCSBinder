//! Safety-critical two-phase commit engine for remapping a chosen binding's
//! content under a new GUID across every aircraft folder.
//!
//! Algorithm summary (full detail in `docs/ARCHITECTURE.md`):
//! 1. Plan (no writes).
//! 2. Manifest written via `tempfile::NamedTempFile::persist()`. Existence = started.
//! 3. Backup (full snapshot of every affected file, blake3-verified).
//! 4. Write (`tmp → fs::rename`, atomic within an NTFS directory).
//! 5. Move-not-delete (stale-GUID files → backup folder).
//! 6. Finalize (`manifest.json.done` sibling marker).
//!
//! On startup, an un-`.done` manifest triggers the rollback prompt.

pub mod execute;
pub mod hash;
pub mod plan;
pub mod recover;
pub mod types;

pub use execute::{execute, ExecuteError};
pub use plan::{plan, plan_with_scope, PlanError};
pub use recover::{recover, undo, IncompleteOperation, RecoverError, UndoError};
pub use types::{BackupEntry, Manifest, Mutation, OperationKind, MANIFEST_VERSION};
