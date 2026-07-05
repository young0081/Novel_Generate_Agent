//! Strongly-typed, process-unique identifiers.
//!
//! IDs combine the current epoch-millis with a monotonic atomic counter so they
//! are unique within a run and roughly sortable by creation time. Each domain
//! gets its own newtype so you cannot accidentally pass a `CheckpointId` where a
//! `SessionId` is expected.

use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

static COUNTER: AtomicU64 = AtomicU64::new(1);

/// Generate a fresh id string with the given prefix, e.g. `sess_19f3a2c0b_0001`.
pub fn next_id(prefix: &str) -> String {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let ts = crate::time::now_millis();
    format!("{prefix}_{ts:x}_{n:04x}")
}

/// Define a transparent `String` newtype with a typed constructor.
macro_rules! typed_id {
    ($(#[$meta:meta])* $name:ident, $prefix:literal) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl $name {
            /// Allocate a brand-new id.
            pub fn new() -> Self {
                $name(next_id($prefix))
            }

            /// Wrap an existing string (e.g. when loading from disk).
            pub fn from_existing(s: impl Into<String>) -> Self {
                $name(s.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl From<String> for $name {
            fn from(s: String) -> Self {
                $name(s)
            }
        }

        impl From<&str> for $name {
            fn from(s: &str) -> Self {
                $name(s.to_string())
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }
    };
}

typed_id!(
    /// Identifies a creation session (one conversation / working context).
    SessionId, "sess"
);
typed_id!(
    /// Identifies a single tool invocation within a session.
    ToolCallId, "call"
);
typed_id!(
    /// Identifies a checkpoint (workspace snapshot).
    CheckpointId, "ckpt"
);
typed_id!(
    /// Identifies a long-term memory entry.
    MemoryId, "mem"
);
typed_id!(
    /// Identifies a single chat message.
    MessageId, "msg"
);

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn ids_are_unique() {
        let mut seen = HashSet::new();
        for _ in 0..1000 {
            assert!(seen.insert(SessionId::new().0));
        }
        assert_eq!(seen.len(), 1000);
    }

    #[test]
    fn prefixes_are_typed() {
        assert!(SessionId::new().as_str().starts_with("sess_"));
        assert!(ToolCallId::new().as_str().starts_with("call_"));
        assert!(CheckpointId::new().as_str().starts_with("ckpt_"));
        assert!(MemoryId::new().as_str().starts_with("mem_"));
    }

    #[test]
    fn round_trips_through_json() {
        let id = MemoryId::new();
        let s = serde_json::to_string(&id).unwrap();
        // transparent => just a quoted string
        assert!(s.starts_with('"'));
        let back: MemoryId = serde_json::from_str(&s).unwrap();
        assert_eq!(id, back);
    }
}
