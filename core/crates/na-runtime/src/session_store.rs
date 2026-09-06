//! A small on-disk store for [`Session`]s, so writing and discussion threads
//! survive restarts and can be browsed + resumed (a "session library").
//!
//! Each session is one JSON file (`<id>.json`) under a directory, wrapped in a
//! [`SessionRecord`] that adds light metadata (its `kind` and originating
//! `goal`). [`list`](SessionStore::list) returns lightweight
//! [`SessionSummary`]s sorted newest-first; [`get`](SessionStore::get) loads a
//! full record to resume from. Files written by an older build that stored a
//! bare [`Session`] are still read (treated as a `writing` record).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use na_common::{CoreError, Result};

use crate::message::Role;
use crate::session::{atomic_write, Session};

/// A persisted session plus light metadata for the library.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionRecord {
    /// The full conversation + agent state.
    pub session: Session,
    /// What kind of session this is: `"writing"`, `"discuss"`, `"planning"`.
    #[serde(default = "default_kind")]
    pub kind: String,
    /// The originating creation goal (for writing sessions), for display/resume.
    #[serde(default)]
    pub goal: Option<String>,
}

fn default_kind() -> String {
    "writing".to_string()
}

/// A lightweight summary of a session for list views.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: String,
    pub title: String,
    pub kind: String,
    pub goal: Option<String>,
    pub messages: usize,
    pub created_ms: u64,
    pub updated_ms: u64,
    /// A short snippet of the latest meaningful content.
    pub preview: String,
}

/// A directory-backed store of [`SessionRecord`]s.
#[derive(Debug, Clone)]
pub struct SessionStore {
    dir: PathBuf,
}

impl SessionStore {
    /// Open (creating if needed) a session store rooted at `dir`.
    pub fn open(dir: impl AsRef<Path>) -> Result<Self> {
        let dir = dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&dir)
            .map_err(|e| CoreError::from(e).with_context("creating sessions directory"))?;
        Ok(SessionStore { dir })
    }

    fn path_for(&self, id: &str) -> Result<PathBuf> {
        validate_session_id(id)?;
        Ok(self.dir.join(format!("{id}.json")))
    }

    /// Insert or replace a session (keyed by its id), atomically.
    pub fn save(&self, record: &SessionRecord) -> Result<()> {
        let id = record.session.id.as_str();
        let path = self.path_for(id)?;
        let json = serde_json::to_string_pretty(record)?;
        atomic_write(&path, json.as_bytes(), "session record")
    }

    /// Load one full record by id.
    pub fn get(&self, id: &str) -> Result<SessionRecord> {
        let path = self.path_for(id)?;
        let text = std::fs::read_to_string(&path)
            .map_err(|e| CoreError::from(e).with_context(format!("reading session {id}")))?;
        let record = parse_record(&text)?;
        validate_record_id(&record, id)?;
        Ok(record)
    }

    /// Remove a session by id (no-op if it doesn't exist).
    pub fn delete(&self, id: &str) -> Result<()> {
        let path = self.path_for(id)?;
        if path.exists() {
            std::fs::remove_file(&path)
                .map_err(|e| CoreError::from(e).with_context("deleting session file"))?;
        }
        Ok(())
    }

    /// All session summaries, newest-updated first.
    pub fn list(&self) -> Result<Vec<SessionSummary>> {
        let mut out = Vec::new();
        let entries = std::fs::read_dir(&self.dir)
            .map_err(|e| CoreError::from(e).with_context("reading sessions directory"))?;
        for entry in entries {
            let entry = entry
                .map_err(|e| CoreError::from(e).with_context("reading sessions directory entry"))?;
            let path = entry.path();
            // Only plain `.json` files (skip temporary and unrelated files).
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let text = std::fs::read_to_string(&path).map_err(|e| {
                CoreError::from(e).with_context(format!("reading session file {}", path.display()))
            })?;
            let rec = parse_record(&text).map_err(|error| {
                error.with_context(format!("parsing session file {}", path.display()))
            })?;
            let file_id = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .ok_or_else(|| {
                    CoreError::new(
                        na_common::ErrorKind::Serialization,
                        format!("session filename is not valid Unicode: {}", path.display()),
                    )
                })?;
            validate_record_id(&rec, file_id)?;
            out.push(summarize(&rec));
        }
        out.sort_by(|a, b| b.updated_ms.cmp(&a.updated_ms));
        Ok(out)
    }
}

fn validate_session_id(id: &str) -> Result<()> {
    if id.is_empty()
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(CoreError::invalid_input(format!(
            "invalid session id {id:?}"
        )));
    }
    Ok(())
}

fn validate_record_id(record: &SessionRecord, expected: &str) -> Result<()> {
    let actual = record.session.id.as_str();
    if actual != expected {
        return Err(CoreError::new(
            na_common::ErrorKind::Serialization,
            format!("session id mismatch: file is {expected:?}, record is {actual:?}"),
        ));
    }
    Ok(())
}

/// Parse a record, tolerating an older bare-[`Session`] file.
fn parse_record(text: &str) -> Result<SessionRecord> {
    if let Ok(rec) = serde_json::from_str::<SessionRecord>(text) {
        return Ok(rec);
    }
    let session = Session::from_json(text)?;
    Ok(SessionRecord {
        session,
        kind: default_kind(),
        goal: None,
    })
}

fn summarize(rec: &SessionRecord) -> SessionSummary {
    let s = &rec.session;
    SessionSummary {
        id: s.id.as_str().to_string(),
        title: s.title.clone(),
        kind: rec.kind.clone(),
        goal: rec.goal.clone(),
        messages: s.messages.len(),
        created_ms: s.created_ms,
        updated_ms: s.updated_ms,
        preview: preview_text(s),
    }
}

/// A short, single-line preview: the latest non-empty assistant turn, else the
/// latest non-empty message of any role.
fn preview_text(s: &Session) -> String {
    for m in s.messages.iter().rev() {
        if m.role == Role::Assistant && !m.content.trim().is_empty() {
            return clip(&m.content, 80);
        }
    }
    for m in s.messages.iter().rev() {
        if !m.content.trim().is_empty() {
            return clip(&m.content, 80);
        }
    }
    String::new()
}

fn clip(s: &str, max: usize) -> String {
    let one_line = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() <= max {
        one_line
    } else {
        let head: String = one_line.chars().take(max).collect();
        format!("{head}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::Message;

    fn temp_dir(tag: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("na_sessions_{}_{}", tag, na_common::next_id("t")));
        p
    }

    fn writing_record(title: &str, goal: &str) -> SessionRecord {
        let mut s = Session::new(title);
        s.push(Message::user(goal));
        s.push(Message::assistant("第一章：风雪夜归人……"));
        SessionRecord {
            session: s,
            kind: "writing".to_string(),
            goal: Some(goal.to_string()),
        }
    }

    #[test]
    fn save_get_round_trips() {
        let store = SessionStore::open(temp_dir("rt")).unwrap();
        let rec = writing_record("北境", "写第一章");
        let id = rec.session.id.as_str().to_string();
        store.save(&rec).unwrap();
        let back = store.get(&id).unwrap();
        assert_eq!(back, rec);
    }

    #[test]
    fn list_sorts_newest_first_and_summarizes() {
        let store = SessionStore::open(temp_dir("list")).unwrap();
        let mut a = writing_record("甲", "写甲");
        a.session.created_ms = 1000;
        a.session.updated_ms = 1000;
        let mut b = writing_record("乙", "写乙");
        b.session.created_ms = 2000;
        b.session.updated_ms = 2000;
        store.save(&a).unwrap();
        store.save(&b).unwrap();

        let list = store.list().unwrap();
        assert_eq!(list.len(), 2);
        // newest (b) first
        assert_eq!(list[0].title, "乙");
        assert_eq!(list[1].title, "甲");
        assert_eq!(list[0].kind, "writing");
        assert_eq!(list[0].messages, 2);
        assert!(list[0].preview.contains("风雪"));
        assert_eq!(list[0].goal.as_deref(), Some("写乙"));
    }

    #[test]
    fn delete_removes() {
        let store = SessionStore::open(temp_dir("del")).unwrap();
        let rec = writing_record("丙", "写丙");
        let id = rec.session.id.as_str().to_string();
        store.save(&rec).unwrap();
        assert_eq!(store.list().unwrap().len(), 1);
        store.delete(&id).unwrap();
        assert_eq!(store.list().unwrap().len(), 0);
        // deleting a missing id is a no-op
        store.delete(&id).unwrap();
    }

    #[test]
    fn reads_legacy_bare_session_file() {
        let dir = temp_dir("legacy");
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = Session::new("旧档");
        s.push(Message::assistant("旧的成稿内容"));
        // Write a bare Session (no record wrapper), as an older build would.
        let path = dir.join(format!("{}.json", s.id.as_str()));
        std::fs::write(&path, s.to_json().unwrap()).unwrap();

        let store = SessionStore::open(&dir).unwrap();
        let list = store.list().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].kind, "writing");
        assert_eq!(list[0].title, "旧档");
        let rec = store.get(s.id.as_str()).unwrap();
        assert_eq!(rec.goal, None);
    }

    #[test]
    fn ids_cannot_escape_the_session_directory() {
        let dir = temp_dir("traversal");
        let store = SessionStore::open(&dir).unwrap();
        let outside = dir.parent().unwrap().join("session-store-outside.json");
        std::fs::write(&outside, "do not delete").unwrap();

        assert!(store.get("../session-store-outside").is_err());
        assert!(store.delete("../session-store-outside").is_err());
        assert_eq!(std::fs::read_to_string(&outside).unwrap(), "do not delete");
        let _ = std::fs::remove_file(outside);
    }

    #[test]
    fn record_id_must_match_requested_filename() {
        let dir = temp_dir("id-mismatch");
        let store = SessionStore::open(&dir).unwrap();
        let session = Session::new("wrong file");
        let actual_id = session.id.as_str().to_string();
        let record = SessionRecord {
            session,
            kind: "writing".into(),
            goal: None,
        };
        std::fs::write(
            dir.join("sess_expected.json"),
            serde_json::to_vec_pretty(&record).unwrap(),
        )
        .unwrap();

        let get_error = store.get("sess_expected").unwrap_err();
        assert!(get_error.is(na_common::ErrorKind::Serialization));
        let list_error = store.list().unwrap_err();
        assert!(list_error.is(na_common::ErrorKind::Serialization));
        assert!(!dir.join(format!("{actual_id}.json")).exists());
    }

    #[test]
    fn corrupt_session_is_reported_by_listing() {
        let dir = temp_dir("corrupt-list");
        let store = SessionStore::open(&dir).unwrap();
        std::fs::write(dir.join("sess_corrupt.json"), "{ broken").unwrap();

        let error = store.list().unwrap_err();
        assert!(error.is(na_common::ErrorKind::Serialization), "{error}");
    }
}
