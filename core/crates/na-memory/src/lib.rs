//! `na-memory` — durable state for the Novel Generate Team core.
//!
//! This crate provides the three "memory" subsystems the agent relies on, all
//! built on `na-common`'s normalized [`na_common::Result`]/[`na_common::CoreError`]
//! contract and plain synchronous file IO (deterministic and easy to reason about):
//!
//! * **Checkpoints** ([`checkpoint`]) — content-addressed snapshots of a whole
//!   workspace directory with byte-exact [`restore`](checkpoint::CheckpointStore::restore)
//!   (including deleting files created after the snapshot) plus
//!   [`undo`](checkpoint::CheckpointStore::undo)/[`redo`](checkpoint::CheckpointStore::redo).
//!   The content key is a length-tagged 64-bit FNV-1a hex digest
//!   ([`content_hash`](checkpoint::content_hash)).
//!
//! * **Audit log** ([`audit`]) — an append-only, thread-safe, structured JSON-Lines
//!   record of every security-relevant event (tool calls, permission decisions,
//!   errors, checkpoints), with a simple [`query`](audit::AuditLog::query) filter.
//!
//! * **Long-term memory RAG** ([`memory`] + [`bm25`]) — a persisted store of story
//!   facts (characters, settings, world rules, plot, foreshadowing, ...) indexed by
//!   a pure-Rust, **CJK-aware** BM25 retriever so the agent can recall the handful
//!   of relevant notes for what it is currently writing. Crucially,
//!   [`recall`](memory::MemoryStore::recall) returns only structured *summaries*
//!   ([`memory::RecallHit`]) so callers do not stuff full content back into the model;
//!   full text is fetched on demand via [`get`](memory::MemoryStore::get).
//!
//! ```
//! use na_memory::{MemoryStore, MemoryKind};
//! # fn demo(path: &std::path::Path) -> na_common::Result<()> {
//! let mut store = MemoryStore::open(path)?;
//! let id = store.save(
//!     MemoryKind::Character,
//!     "林惊羽",
//!     "冷静的年轻剑客，主角。",
//!     "林惊羽出身寒门，使一柄名为‘霜寒’的长剑。",
//!     vec!["主角".into(), "剑客".into()],
//!     5,
//! )?;
//! // Recall surfaces only the summary header, not the full content.
//! let hits = store.recall("剑客", 5, None, false);
//! assert_eq!(hits[0].id, id);
//! # Ok(())
//! # }
//! ```

#![forbid(unsafe_code)]

pub mod audit;
pub mod bm25;
pub mod checkpoint;
pub mod memory;

// ---- Re-exports for an ergonomic top-level API ----

pub use audit::{AuditEntry, AuditFilter, AuditLog};
pub use bm25::Bm25Index;
pub use checkpoint::{content_hash, CheckpointManifest, CheckpointMeta, CheckpointStore};
pub use memory::{
    tokenize, Bm25Retriever, Embedder, MemoryEntry, MemoryKind, MemoryStore, RecallHit, Retriever,
};

// Re-export the common id/result types most callers of this crate will need, so
// they don't have to also reach into `na_common` for the basics.
pub use na_common::{CheckpointId, CoreError, MemoryId, Result};
