//! `na-library` — multi-work management + per-work knowledge bases.
//!
//! This crate adds two libraries on top of the single-workspace core:
//!
//! * **Works** ([`work`]) — a "book library": the user can keep many separate
//!   novels, each fully isolated in its own directory (manuscript, memory,
//!   checkpoints, sessions, story-state, knowledge bases). [`WorkStore`] tracks
//!   them in an index file and remembers which one is active. The GUI rebuilds
//!   the core engine pointed at the active work's workspace, so switching books
//!   is total isolation with zero cross-contamination.
//!
//! * **Knowledge bases** ([`knowledge`]) — per-work RAG corpora. A work can own
//!   several [`KnowledgeBase`]s (e.g. one auto-filled from the source material,
//!   one hand-curated). Each is a JSON-Lines file of [`KnowledgeEntry`]s indexed
//!   by the same CJK-aware BM25 retriever the long-term memory uses, so the agent
//!   can pull the handful of canon facts relevant to what it's writing and stay
//!   on-setting. Knowledge bases can be marked *active* for retrieval, and the
//!   set of active bases is what RAG injection searches.
//!
//! Everything is plain synchronous file IO with atomic writes, mirroring the
//! conventions in `na-memory` so the whole core stays deterministic and testable.

#![forbid(unsafe_code)]

pub mod knowledge;
pub mod work;

pub use knowledge::{
    KbId, KnowledgeBase, KnowledgeBaseMeta, KnowledgeEntry, KnowledgeHit, KnowledgeKind,
    KnowledgeStore,
};
pub use work::{WorkId, WorkMeta, WorkStore, WorkSummary};

pub use na_common::{CoreError, Result};
