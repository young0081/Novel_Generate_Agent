//! Append-only, structured audit log (JSON Lines).
//!
//! Every security-relevant event — a tool call, a permission decision, an error,
//! a checkpoint — is appended as one self-contained JSON object per line. This
//! format is trivially greppable, append-only (so history cannot be silently
//! rewritten), and survives partial writes (a torn last line just fails to parse
//! and is skipped on read).
//!
//! Writes are serialized through a [`Mutex`] so the log is safe to share across
//! threads/tasks (`AuditLog` is `Send + Sync`). Queries read the whole file and
//! filter in memory, which is fine for the volumes a single creation session
//! produces.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use na_common::time::{format_utc, now_millis};
use na_common::{CoreError, Json, Result};
use serde::{Deserialize, Serialize};

/// One structured audit record.
///
/// Construct via [`AuditEntry::new`] and chain the setters; the timestamp fields
/// are filled in automatically (overridable with [`AuditEntry::at`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuditEntry {
    /// Epoch milliseconds when the event occurred.
    pub ts_ms: u64,
    /// Human-readable ISO-8601 UTC rendering of `ts_ms`.
    pub ts_iso: String,
    /// Owning session id, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    /// Event category, e.g. `"tool_call"`, `"permission"`, `"error"`, `"checkpoint"`.
    pub event: String,
    /// Tool name, when the event concerns a tool.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    /// Decision for permission events, e.g. `"allow"`, `"deny"`, `"prompt"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<String>,
    /// Whether the operation succeeded.
    pub ok: bool,
    /// Stable error code (mirrors [`na_common::ErrorKind::code`]) on failure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    /// Arbitrary structured detail (arguments, byte counts, paths, ...).
    #[serde(default, skip_serializing_if = "Json::is_null")]
    pub detail: Json,
}

impl AuditEntry {
    /// Start a new entry for `event`, stamped with the current time and `ok = true`.
    pub fn new(event: impl Into<String>) -> Self {
        let ts_ms = now_millis();
        AuditEntry {
            ts_ms,
            ts_iso: format_utc(ts_ms),
            session: None,
            event: event.into(),
            tool: None,
            decision: None,
            ok: true,
            error_code: None,
            detail: Json::Null,
        }
    }

    /// Override the timestamp (both the millis and ISO fields).
    pub fn at(mut self, ts_ms: u64) -> Self {
        self.ts_ms = ts_ms;
        self.ts_iso = format_utc(ts_ms);
        self
    }

    /// Attach a session id.
    pub fn session(mut self, session: impl Into<String>) -> Self {
        self.session = Some(session.into());
        self
    }

    /// Attach a tool name.
    pub fn tool(mut self, tool: impl Into<String>) -> Self {
        self.tool = Some(tool.into());
        self
    }

    /// Attach a permission/decision label.
    pub fn decision(mut self, decision: impl Into<String>) -> Self {
        self.decision = Some(decision.into());
        self
    }

    /// Set the success flag.
    pub fn ok(mut self, ok: bool) -> Self {
        self.ok = ok;
        self
    }

    /// Attach an error code and implicitly mark the entry as failed.
    pub fn error_code(mut self, code: impl Into<String>) -> Self {
        self.error_code = Some(code.into());
        self.ok = false;
        self
    }

    /// Record failure directly from a [`CoreError`]: sets `ok = false` and copies
    /// the stable code.
    pub fn from_error(mut self, err: &CoreError) -> Self {
        self.ok = false;
        self.error_code = Some(err.code.clone());
        self
    }

    /// Attach an arbitrary structured detail payload.
    pub fn detail(mut self, detail: Json) -> Self {
        self.detail = detail;
        self
    }
}

/// Optional filter for [`AuditLog::query`]. All set fields must match (logical
/// AND); unset fields match anything.
#[derive(Debug, Clone, Default)]
pub struct AuditFilter {
    pub event: Option<String>,
    pub tool: Option<String>,
    pub ok: Option<bool>,
    pub session: Option<String>,
}

impl AuditFilter {
    /// An empty filter that matches every entry.
    pub fn new() -> Self {
        Self::default()
    }
    pub fn event(mut self, event: impl Into<String>) -> Self {
        self.event = Some(event.into());
        self
    }
    pub fn tool(mut self, tool: impl Into<String>) -> Self {
        self.tool = Some(tool.into());
        self
    }
    pub fn ok(mut self, ok: bool) -> Self {
        self.ok = Some(ok);
        self
    }
    pub fn session(mut self, session: impl Into<String>) -> Self {
        self.session = Some(session.into());
        self
    }

    fn matches(&self, e: &AuditEntry) -> bool {
        if let Some(ref ev) = self.event {
            if &e.event != ev {
                return false;
            }
        }
        if let Some(ref tool) = self.tool {
            if e.tool.as_deref() != Some(tool.as_str()) {
                return false;
            }
        }
        if let Some(ok) = self.ok {
            if e.ok != ok {
                return false;
            }
        }
        if let Some(ref s) = self.session {
            if e.session.as_deref() != Some(s.as_str()) {
                return false;
            }
        }
        true
    }
}

/// A thread-safe handle to an append-only audit log file.
pub struct AuditLog {
    path: PathBuf,
    /// Guards the file so concurrent `record` calls do not interleave lines.
    write_lock: Mutex<()>,
}

impl AuditLog {
    /// Open (creating parent directories as needed) the log at `path`. The file
    /// itself is created lazily on the first [`record`](Self::record).
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)
                    .map_err(|e| CoreError::from(e).with_context("creating audit log directory"))?;
            }
        }
        Ok(AuditLog {
            path,
            write_lock: Mutex::new(()),
        })
    }

    /// The file path backing this log.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Append one entry as a single JSON line. Thread-safe.
    pub fn record(&self, entry: AuditEntry) -> Result<()> {
        let line = serde_json::to_string(&entry)?;
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| CoreError::internal("audit log write lock poisoned"))?;
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|e| CoreError::from(e).with_context("opening audit log for append"))?;
        file.write_all(line.as_bytes())
            .and_then(|_| file.write_all(b"\n"))
            .map_err(|e| CoreError::from(e).with_context("appending audit entry"))?;
        Ok(())
    }

    /// Convenience: build an entry, apply `f`, and record it.
    pub fn record_with(
        &self,
        event: impl Into<String>,
        f: impl FnOnce(AuditEntry) -> AuditEntry,
    ) -> Result<()> {
        self.record(f(AuditEntry::new(event)))
    }

    /// Read every entry, in file order, that matches `filter`. A missing file
    /// yields an empty vector. Lines that fail to parse (e.g. a torn final write)
    /// are skipped rather than aborting the whole query.
    pub fn query(&self, filter: AuditFilter) -> Result<Vec<AuditEntry>> {
        self.read_all_filtered(&filter, true)
    }

    /// Like [`query`](Self::query) but treats a malformed line as an error
    /// instead of skipping it. Useful in tests / integrity checks.
    pub fn query_strict(&self, filter: AuditFilter) -> Result<Vec<AuditEntry>> {
        self.read_all_filtered(&filter, false)
    }

    fn read_all_filtered(&self, filter: &AuditFilter, lenient: bool) -> Result<Vec<AuditEntry>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| CoreError::internal("audit log lock poisoned"))?;
        let file = fs::File::open(&self.path)
            .map_err(|e| CoreError::from(e).with_context("opening audit log for read"))?;
        let reader = BufReader::new(file);
        let mut out = Vec::new();
        for (lineno, line) in reader.lines().enumerate() {
            let line =
                line.map_err(|e| CoreError::from(e).with_context("reading audit log line"))?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            match serde_json::from_str::<AuditEntry>(trimmed) {
                Ok(entry) => {
                    if filter.matches(&entry) {
                        out.push(entry);
                    }
                }
                Err(e) => {
                    if lenient {
                        continue;
                    }
                    return Err(CoreError::from(e)
                        .with_context(format!("parsing audit log line {}", lineno + 1)));
                }
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use na_common::json;
    use std::path::PathBuf;

    struct TempDir {
        path: PathBuf,
    }
    impl TempDir {
        fn new() -> Self {
            let mut p = std::env::temp_dir();
            p.push(format!("na_memory_audit_{}", na_common::next_id("a")));
            fs::create_dir_all(&p).unwrap();
            TempDir { path: p }
        }
        fn file(&self, name: &str) -> PathBuf {
            self.path.join(name)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn records_and_reads_back() {
        let td = TempDir::new();
        let log = AuditLog::open(td.file("audit.jsonl")).unwrap();
        log.record(
            AuditEntry::new("tool_call")
                .tool("write_file")
                .session("sess1")
                .detail(json!({ "path": "ch01.md", "bytes": 1234 })),
        )
        .unwrap();
        log.record(
            AuditEntry::new("permission")
                .decision("allow")
                .tool("shell"),
        )
        .unwrap();

        let all = log.query(AuditFilter::new()).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].event, "tool_call");
        assert_eq!(all[0].detail["bytes"], 1234);
        assert!(all[0].ts_iso.ends_with('Z'));
    }

    #[test]
    fn filters_by_event_tool_ok_session() {
        let td = TempDir::new();
        let log = AuditLog::open(td.file("a.jsonl")).unwrap();
        log.record(AuditEntry::new("tool_call").tool("read_file").session("s1"))
            .unwrap();
        log.record(
            AuditEntry::new("tool_call")
                .tool("write_file")
                .session("s1")
                .ok(false)
                .error_code("permission_denied"),
        )
        .unwrap();
        log.record(AuditEntry::new("error").session("s2").ok(false))
            .unwrap();

        assert_eq!(
            log.query(AuditFilter::new().event("tool_call"))
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            log.query(AuditFilter::new().tool("write_file"))
                .unwrap()
                .len(),
            1
        );
        assert_eq!(log.query(AuditFilter::new().ok(false)).unwrap().len(), 2);
        assert_eq!(log.query(AuditFilter::new().ok(true)).unwrap().len(), 1);
        assert_eq!(
            log.query(AuditFilter::new().session("s2")).unwrap().len(),
            1
        );

        // combined AND filter
        let combo = log
            .query(
                AuditFilter::new()
                    .event("tool_call")
                    .session("s1")
                    .ok(false),
            )
            .unwrap();
        assert_eq!(combo.len(), 1);
        assert_eq!(combo[0].tool.as_deref(), Some("write_file"));
        assert_eq!(combo[0].error_code.as_deref(), Some("permission_denied"));
    }

    #[test]
    fn missing_file_yields_empty() {
        let td = TempDir::new();
        let log = AuditLog::open(td.file("nope.jsonl")).unwrap();
        assert!(log.query(AuditFilter::new()).unwrap().is_empty());
    }

    #[test]
    fn from_error_sets_ok_false_and_code() {
        let td = TempDir::new();
        let log = AuditLog::open(td.file("e.jsonl")).unwrap();
        let err = CoreError::sandbox("escape attempt");
        log.record(AuditEntry::new("error").tool("shell").from_error(&err))
            .unwrap();
        let got = log.query(AuditFilter::new().ok(false)).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].error_code.as_deref(), Some("sandbox_violation"));
    }

    #[test]
    fn lenient_query_skips_corrupt_line() {
        let td = TempDir::new();
        let path = td.file("torn.jsonl");
        let log = AuditLog::open(&path).unwrap();
        log.record(AuditEntry::new("ok_event")).unwrap();
        // simulate a torn write by appending a half-written line
        {
            let mut f = fs::OpenOptions::new().append(true).open(&path).unwrap();
            f.write_all(b"{ this is not valid json\n").unwrap();
        }
        log.record(AuditEntry::new("after")).unwrap();

        // lenient query skips the bad line
        let all = log.query(AuditFilter::new()).unwrap();
        assert_eq!(all.len(), 2);
        // strict query errors out
        assert!(log.query_strict(AuditFilter::new()).is_err());
    }

    #[test]
    fn record_with_helper_works() {
        let td = TempDir::new();
        let log = AuditLog::open(td.file("h.jsonl")).unwrap();
        log.record_with("checkpoint", |e| e.detail(json!({ "id": "ckpt_1" })))
            .unwrap();
        let all = log.query(AuditFilter::new().event("checkpoint")).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].detail["id"], "ckpt_1");
    }

    #[test]
    fn concurrent_records_do_not_corrupt() {
        use std::sync::Arc;
        use std::thread;
        let td = TempDir::new();
        let log = Arc::new(AuditLog::open(td.file("c.jsonl")).unwrap());
        let mut handles = Vec::new();
        for t in 0..8 {
            let log = Arc::clone(&log);
            handles.push(thread::spawn(move || {
                for i in 0..50 {
                    log.record(
                        AuditEntry::new("tool_call")
                            .tool("t")
                            .detail(json!({ "thread": t, "i": i })),
                    )
                    .unwrap();
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        // every one of the 8*50 lines must parse cleanly (no interleaving)
        let all = log.query_strict(AuditFilter::new()).unwrap();
        assert_eq!(all.len(), 400);
    }
}
