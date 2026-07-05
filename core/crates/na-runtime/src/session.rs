//! A creation session: an ordered conversation plus opaque agent state,
//! persistable to disk.
//!
//! A [`Session`] is the durable spine of a writing run. It owns the full
//! [`Message`] history, a free-form `state` JSON blob the agent can stash
//! arbitrary scratch data in (current chapter, plan, counters, ...), and
//! creation / update timestamps. It serializes losslessly so a run can be
//! suspended to a file and resumed later (see [`save`](Session::save) /
//! [`load`](Session::load)).

use std::path::Path;

use na_common::time::now_millis;
use na_common::{CoreError, Json, Result, SessionId};
use serde::{Deserialize, Serialize};

use crate::message::Message;

/// A persistable conversation + agent state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Session {
    /// Stable session id.
    pub id: SessionId,
    /// Human-readable title (e.g. the book / arc name).
    pub title: String,
    /// The full ordered message history.
    pub messages: Vec<Message>,
    /// Opaque agent state (plan, counters, current chapter, ...). Defaults to
    /// `null` for a fresh session.
    pub state: Json,
    /// Creation time (epoch millis).
    pub created_ms: u64,
    /// Last-mutation time (epoch millis).
    pub updated_ms: u64,
}

impl Session {
    /// Create a fresh, empty session with the given title.
    pub fn new(title: impl Into<String>) -> Self {
        let now = now_millis();
        Session {
            id: SessionId::new(),
            title: title.into(),
            messages: Vec::new(),
            state: Json::Null,
            created_ms: now,
            updated_ms: now,
        }
    }

    /// Append a message and bump the update timestamp.
    pub fn push(&mut self, message: Message) {
        self.messages.push(message);
        self.touch();
    }

    /// Append several messages at once.
    pub fn extend<I: IntoIterator<Item = Message>>(&mut self, messages: I) {
        let before = self.messages.len();
        self.messages.extend(messages);
        if self.messages.len() != before {
            self.touch();
        }
    }

    /// The full message history (read-only).
    pub fn history(&self) -> &[Message] {
        &self.messages
    }

    /// The most recent message, if any.
    pub fn last(&self) -> Option<&Message> {
        self.messages.last()
    }

    /// Number of messages.
    pub fn len(&self) -> usize {
        self.messages.len()
    }

    /// Whether the history is empty.
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    /// Replace the agent state blob and bump the update timestamp.
    pub fn set_state(&mut self, state: Json) {
        self.state = state;
        self.touch();
    }

    /// Update the `updated_ms` timestamp to now.
    pub fn touch(&mut self) {
        self.updated_ms = now_millis();
    }

    /// Serialize the session to pretty JSON.
    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    /// Parse a session from JSON.
    pub fn from_json(json: &str) -> Result<Self> {
        Ok(serde_json::from_str(json)?)
    }

    /// Save the session to `path` (pretty JSON), creating parent directories.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| CoreError::from(e).with_context("creating session directory"))?;
            }
        }
        let json = self.to_json()?;
        // Write to a temp file then rename for atomicity.
        let tmp = with_tmp_extension(path);
        std::fs::write(&tmp, json.as_bytes())
            .map_err(|e| CoreError::from(e).with_context("writing session file"))?;
        std::fs::rename(&tmp, path)
            .map_err(|e| CoreError::from(e).with_context("replacing session file"))?;
        Ok(())
    }

    /// Load a session from `path`.
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let bytes = std::fs::read(path).map_err(|e| {
            CoreError::from(e).with_context(format!("reading session file {}", path.display()))
        })?;
        let text = String::from_utf8(bytes)
            .map_err(|e| CoreError::new(na_common::ErrorKind::Serialization, e.to_string()))?;
        Session::from_json(&text)
    }
}

/// Build a sibling temp path next to `path` for atomic writes.
fn with_tmp_extension(path: &Path) -> std::path::PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(".tmp");
    std::path::PathBuf::from(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{Message, Role, ToolCallRequest, ToolResultRef};
    use na_common::json;
    use na_common::ToolCallId;

    fn temp_path(tag: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "na_runtime_session_{}_{}",
            tag,
            na_common::next_id("t")
        ));
        std::fs::create_dir_all(&p).unwrap();
        p.join("session.json")
    }

    #[test]
    fn new_session_is_empty_with_timestamps() {
        let s = Session::new("狼人杀");
        assert_eq!(s.title, "狼人杀");
        assert!(s.is_empty());
        assert!(s.created_ms > 0);
        assert_eq!(s.created_ms, s.updated_ms);
        assert!(s.id.as_str().starts_with("sess_"));
    }

    #[test]
    fn push_appends_and_touches() {
        let mut s = Session::new("t");
        let before = s.updated_ms;
        std::thread::sleep(std::time::Duration::from_millis(2));
        s.push(Message::user("hi"));
        assert_eq!(s.len(), 1);
        assert_eq!(s.history()[0].content, "hi");
        assert!(s.updated_ms >= before);
        assert_eq!(s.last().unwrap().role, Role::User);
    }

    #[test]
    fn set_state_stores_blob() {
        let mut s = Session::new("t");
        s.set_state(json!({ "chapter": 3, "plan": ["intro", "climax"] }));
        assert_eq!(s.state["chapter"], 3);
    }

    #[test]
    fn save_then_load_round_trips_equal() {
        let path = temp_path("roundtrip");
        let mut s = Session::new("修仙长篇");
        s.push(Message::system("你是一名小说家。"));
        s.push(Message::user("写第一章"));
        let call = ToolCallRequest::with_id(
            ToolCallId::from_existing("call_x"),
            "write_file",
            json!({ "path": "ch1.md", "content": "第一章" }),
        );
        s.push(Message::assistant_tool_call("我来写", call.clone()));
        s.push(Message::tool(
            "wrote 3 bytes",
            ToolResultRef::new(call.id.clone(), "write_file", true, false),
        ));
        s.set_state(json!({ "words": 3 }));

        s.save(&path).unwrap();
        let loaded = Session::load(&path).unwrap();
        assert_eq!(s, loaded, "session must round-trip losslessly");
        assert_eq!(loaded.messages.len(), 4);
        assert_eq!(loaded.state["words"], 3);
    }

    #[test]
    fn load_missing_file_is_error() {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "na_runtime_missing_{}.json",
            na_common::next_id("t")
        ));
        let err = Session::load(&p).unwrap_err();
        assert!(err.is(na_common::ErrorKind::NotFound) || err.is(na_common::ErrorKind::Io));
    }

    #[test]
    fn to_from_json_round_trip() {
        let mut s = Session::new("t");
        s.push(Message::assistant("done"));
        let j = s.to_json().unwrap();
        let back = Session::from_json(&j).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn extend_adds_many() {
        let mut s = Session::new("t");
        s.extend([Message::user("a"), Message::assistant("b")]);
        assert_eq!(s.len(), 2);
    }
}
