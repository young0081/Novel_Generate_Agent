//! `na-common` — foundational types shared by every layer of the Novel Generate
//! Team core.
//!
//! Everything here is dependency-light and stable: a single normalized error
//! type ([`CoreError`]), strongly-typed ids, a hierarchical
//! [`CancellationToken`], and tiny time helpers. Downstream crates
//! (`na-sandbox`, `na-tools`, `na-memory`, `na-runtime`) build on these
//! contracts, so this crate must stay coherent and well-tested.

pub mod cancel;
pub mod error;
pub mod id;
pub mod time;

pub use cancel::CancellationToken;
pub use error::{CoreError, ErrorKind, Result};
pub use id::{next_id, CheckpointId, MemoryId, MessageId, SessionId, ToolCallId};

/// The canonical dynamic value type used for tool arguments and results.
pub use serde_json::Value as Json;
pub use serde_json::{json, Map as JsonMap};
