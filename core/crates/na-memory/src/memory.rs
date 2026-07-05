//! Long-term memory store with BM25 retrieval, tuned for a Chinese (and English)
//! novel-writing assistant.
//!
//! The agent accumulates durable facts about the story it is writing — character
//! sheets, settings, world rules, plot beats, foreshadowing to pay off later. We
//! persist these as [`MemoryEntry`] records in a JSON-Lines file and build an
//! in-memory [`Bm25Index`] over their text so the agent can *recall* the few
//! entries relevant to what it is currently writing.
//!
//! ## Why recall returns summaries, not full content
//!
//! [`MemoryStore::recall`] deliberately returns only the structured header of a
//! hit ([`RecallHit`]: title, summary, tags, importance, score) — never the full
//! `content`. The point of the store is to *reduce* what gets stuffed back into
//! the model's context window: the agent skims summaries, and only pulls a full
//! entry with [`MemoryStore::get`] when it explicitly needs the detail.
//!
//! ## Chinese support
//!
//! Naïve whitespace tokenization fails on Chinese, which is not space-delimited.
//! [`tokenize`] emits, for each run of CJK characters, both every single
//! character *and* every adjacent-character bigram, which gives solid lexical
//! recall without a dictionary-based word segmenter.

use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use na_common::time::now_millis;
use na_common::{CoreError, MemoryId, Result};
use serde::{Deserialize, Serialize};

use crate::bm25::Bm25Index;

/// What a memory is *about*. Lets the agent filter recall by category (e.g. only
/// character notes) and helps the UI group entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryKind {
    /// A character: who they are, voice, relationships, arc.
    Character,
    /// A place / scene setting.
    Setting,
    /// World rules, magic systems, factions, history.
    Worldbuilding,
    /// A plot point or event that happened / will happen.
    Plot,
    /// A structural outline (book / arc / chapter scaffold).
    Outline,
    /// A planted detail to be paid off later.
    Foreshadow,
    /// A notable line of dialogue or a character's verbal tic.
    Dialogue,
    /// General lore / trivia about the world.
    Lore,
    /// Anything that does not fit the above.
    Other,
}

impl MemoryKind {
    /// All variants, for UIs and exhaustiveness in tests.
    pub fn all() -> &'static [MemoryKind] {
        use MemoryKind::*;
        &[
            Character,
            Setting,
            Worldbuilding,
            Plot,
            Outline,
            Foreshadow,
            Dialogue,
            Lore,
            Other,
        ]
    }
}

/// A single durable memory record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub id: MemoryId,
    pub kind: MemoryKind,
    /// Short label, e.g. a character or place name.
    pub title: String,
    /// One-to-few sentence summary — this is what recall surfaces.
    pub summary: String,
    /// The full detail. Only returned by [`MemoryStore::get`], never by recall.
    pub content: String,
    /// Free-form tags for filtering / cross-referencing.
    pub tags: Vec<String>,
    /// Subjective importance, clamped to `1..=5` (5 = most important).
    pub importance: u8,
    pub created_ms: u64,
    pub updated_ms: u64,
    /// Archived entries are hidden from recall unless explicitly included.
    pub archived: bool,
}

/// A retrieval result. Carries only the structured header — *not* the full
/// `content` — so callers don't blow up the model's context.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecallHit {
    pub id: MemoryId,
    pub kind: MemoryKind,
    pub title: String,
    pub summary: String,
    pub tags: Vec<String>,
    pub importance: u8,
    /// BM25 relevance score of this hit for the query.
    pub score: f32,
}

/// Tokenize `text` for indexing and querying, with first-class CJK support.
///
/// Rules:
/// * Everything is lowercased.
/// * Maximal runs of ASCII alphanumerics become a single whole-word token
///   (`"Dragon-Lord"` -> `["dragon", "lord"]`).
/// * For runs of CJK characters we emit **both** every single character **and**
///   every adjacent-character bigram (`"龙王"` -> `["龙", "王", "龙王"]`). The
///   bigrams capture short words/phrases; the unigrams keep recall high.
/// * All other characters (punctuation, whitespace, emoji) are separators.
///
/// A character is treated as CJK if it falls in the common CJK Unified
/// Ideographs block or its extension A, plus the CJK compatibility ideographs —
/// which covers virtually all Chinese used in prose.
pub fn tokenize(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut ascii_run = String::new();
    // Buffer of consecutive CJK chars so we can emit unigrams + bigrams.
    let mut cjk_run: Vec<char> = Vec::new();

    fn flush_ascii(run: &mut String, out: &mut Vec<String>) {
        if !run.is_empty() {
            out.push(std::mem::take(run));
        }
    }
    fn flush_cjk(run: &mut Vec<char>, out: &mut Vec<String>) {
        if run.is_empty() {
            return;
        }
        // unigrams
        for &c in run.iter() {
            out.push(c.to_string());
        }
        // adjacent bigrams
        for pair in run.windows(2) {
            let mut s = String::with_capacity(8);
            s.push(pair[0]);
            s.push(pair[1]);
            out.push(s);
        }
        run.clear();
    }

    for ch in text.chars() {
        let lower = ch.to_ascii_lowercase();
        if lower.is_ascii_alphanumeric() {
            flush_cjk(&mut cjk_run, &mut tokens);
            ascii_run.push(lower);
        } else if is_cjk(ch) {
            flush_ascii(&mut ascii_run, &mut tokens);
            cjk_run.push(ch);
        } else {
            // separator
            flush_ascii(&mut ascii_run, &mut tokens);
            flush_cjk(&mut cjk_run, &mut tokens);
        }
    }
    flush_ascii(&mut ascii_run, &mut tokens);
    flush_cjk(&mut cjk_run, &mut tokens);
    tokens
}

/// Whether `c` is a CJK ideograph we should split character-wise.
fn is_cjk(c: char) -> bool {
    let u = c as u32;
    // CJK Unified Ideographs
    (0x4E00..=0x9FFF).contains(&u)
        // CJK Unified Ideographs Extension A
        || (0x3400..=0x4DBF).contains(&u)
        // CJK Compatibility Ideographs
        || (0xF900..=0xFAFF).contains(&u)
        // CJK Unified Ideographs Extension B (astral plane)
        || (0x20000..=0x2A6DF).contains(&u)
}

/// A text -> dense vector embedder. Reserved for a future semantic/vector
/// retriever; not used by the default BM25 path, but part of the public API so
/// downstream crates can plug one in without a breaking change.
pub trait Embedder: Send + Sync {
    /// Map `text` to a fixed-length embedding vector.
    fn embed(&self, text: &str) -> Vec<f32>;
}

/// An object-safe document retriever. The store ships with a BM25 implementation
/// ([`Bm25Retriever`]); a vector-based one can be added later behind the same
/// trait. Object-safe (`dyn Retriever`) and `Send + Sync` so it can live behind a
/// trait object in the runtime.
pub trait Retriever: Send + Sync {
    /// Index a document under `doc_id` from its raw `text`.
    fn index(&mut self, doc_id: &str, text: &str);
    /// Return up to `k` `(doc_id, score)` hits for `query`, best first.
    fn retrieve(&self, query: &str, k: usize) -> Vec<(String, f32)>;
    /// Drop all indexed documents (used before a full rebuild).
    fn clear(&mut self);
}

/// The default BM25-backed [`Retriever`], using the CJK-aware [`tokenize`].
#[derive(Debug, Default)]
pub struct Bm25Retriever {
    index: Bm25Index,
}

impl Bm25Retriever {
    pub fn new() -> Self {
        Bm25Retriever {
            index: Bm25Index::new(),
        }
    }
}

impl Retriever for Bm25Retriever {
    fn index(&mut self, doc_id: &str, text: &str) {
        let tokens = tokenize(text);
        self.index.add(doc_id.to_string(), &tokens);
    }

    fn retrieve(&self, query: &str, k: usize) -> Vec<(String, f32)> {
        let q = tokenize(query);
        self.index.search(&q, k)
    }

    fn clear(&mut self) {
        self.index = Bm25Index::new();
    }
}

/// Persisted long-term memory store with in-memory BM25 retrieval.
///
/// Entries live in a JSON-Lines file; the index is rebuilt from disk on
/// [`open`](Self::open) and kept in sync on mutation. Because BM25 statistics
/// (document frequencies, average length) depend on the whole corpus, mutations
/// that change an entry's text rebuild the index from the current entry set —
/// correct and simple at our scale.
pub struct MemoryStore {
    path: PathBuf,
    entries: Vec<MemoryEntry>,
    /// id string -> index into `entries`, for O(1) lookup.
    by_id: HashMap<String, usize>,
    index: Bm25Index,
}

impl MemoryStore {
    /// Open (or create) the store at `path`, loading any existing entries and
    /// building the retrieval index over them.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent).map_err(|e| {
                    CoreError::from(e).with_context("creating memory store directory")
                })?;
            }
        }
        let entries = load_entries(&path)?;
        let mut store = MemoryStore {
            path,
            entries,
            by_id: HashMap::new(),
            index: Bm25Index::new(),
        };
        store.rebuild();
        Ok(store)
    }

    /// Persist a new memory, index it, and return its fresh id. `importance` is
    /// clamped into `1..=5`.
    pub fn save(
        &mut self,
        kind: MemoryKind,
        title: impl Into<String>,
        summary: impl Into<String>,
        content: impl Into<String>,
        tags: Vec<String>,
        importance: u8,
    ) -> Result<MemoryId> {
        let now = now_millis();
        let id = MemoryId::new();
        let entry = MemoryEntry {
            id: id.clone(),
            kind,
            title: title.into(),
            summary: summary.into(),
            content: content.into(),
            tags,
            importance: importance.clamp(1, 5),
            created_ms: now,
            updated_ms: now,
            archived: false,
        };
        // Append to disk first; only mutate memory if that succeeded.
        self.append_entry(&entry)?;
        let idx = self.entries.len();
        self.by_id.insert(entry.id.0.clone(), idx);
        self.index
            .add(entry.id.0.clone(), &tokenize(&doc_text(&entry)));
        self.entries.push(entry);
        Ok(id)
    }

    /// Retrieve up to `k` entries matching `query`, best first. Returns only
    /// structured [`RecallHit`] headers — **never** full content.
    ///
    /// * `kind_filter`: restrict to a single [`MemoryKind`] when `Some`.
    /// * `include_archived`: archived entries are excluded unless this is `true`.
    pub fn recall(
        &self,
        query: &str,
        k: usize,
        kind_filter: Option<MemoryKind>,
        include_archived: bool,
    ) -> Vec<RecallHit> {
        if k == 0 {
            return Vec::new();
        }
        let q = tokenize(query);
        if q.is_empty() {
            return Vec::new();
        }
        // Over-fetch so that post-filtering (kind/archived) can still return up to
        // k survivors. Cap the over-fetch at the corpus size.
        let want = k.saturating_mul(4).max(k);
        let fetch = want.min(self.entries.len().max(1));
        let raw = self.index.search(&q, fetch.max(k));

        let mut hits = Vec::with_capacity(k);
        for (doc_id, score) in raw {
            let Some(&idx) = self.by_id.get(&doc_id) else {
                continue;
            };
            let e = &self.entries[idx];
            if !include_archived && e.archived {
                continue;
            }
            if let Some(kf) = kind_filter {
                if e.kind != kf {
                    continue;
                }
            }
            hits.push(RecallHit {
                id: e.id.clone(),
                kind: e.kind,
                title: e.title.clone(),
                summary: e.summary.clone(),
                tags: e.tags.clone(),
                importance: e.importance,
                score,
            });
            if hits.len() == k {
                break;
            }
        }
        hits
    }

    /// Fetch a full entry by id (the only API that exposes `content`).
    pub fn get(&self, id: &MemoryId) -> Option<&MemoryEntry> {
        self.by_id.get(&id.0).map(|&i| &self.entries[i])
    }

    /// All entries, in insertion order (read-only).
    pub fn all(&self) -> &[MemoryEntry] {
        &self.entries
    }

    /// Reclassify an entry: optionally change its [`MemoryKind`] and/or append
    /// tags (deduplicated). Persists the whole store and re-indexes (tags are
    /// part of the indexed text). Returns `NotFound` if `id` is unknown.
    pub fn classify(
        &mut self,
        id: &MemoryId,
        new_kind: Option<MemoryKind>,
        add_tags: Vec<String>,
    ) -> Result<()> {
        let idx = *self
            .by_id
            .get(&id.0)
            .ok_or_else(|| CoreError::not_found(format!("memory {id} not found")))?;
        {
            let e = &mut self.entries[idx];
            if let Some(k) = new_kind {
                e.kind = k;
            }
            for t in add_tags {
                if !e.tags.contains(&t) {
                    e.tags.push(t);
                }
            }
            e.updated_ms = now_millis();
        }
        self.persist_all()?;
        self.rebuild();
        Ok(())
    }

    /// Set the archived flag on an entry and persist. Archived entries drop out
    /// of [`recall`](Self::recall) unless `include_archived` is set.
    pub fn archive(&mut self, id: &MemoryId, archived: bool) -> Result<()> {
        let idx = *self
            .by_id
            .get(&id.0)
            .ok_or_else(|| CoreError::not_found(format!("memory {id} not found")))?;
        let changed = {
            let e = &mut self.entries[idx];
            let changed = e.archived != archived;
            e.archived = archived;
            if changed {
                e.updated_ms = now_millis();
            }
            changed
        };
        if changed {
            self.persist_all()?;
            // index membership is unaffected by archiving; no rebuild needed.
        }
        Ok(())
    }

    /// Permanently remove an entry by id, rewrite the store, and re-index.
    /// Returns `NotFound` if `id` is unknown.
    pub fn delete(&mut self, id: &MemoryId) -> Result<()> {
        let idx = *self
            .by_id
            .get(&id.0)
            .ok_or_else(|| CoreError::not_found(format!("memory {id} not found")))?;
        self.entries.remove(idx);
        self.persist_all()?;
        // indices shifted by the removal — rebuild the id map + BM25 index.
        self.rebuild();
        Ok(())
    }

    /// Number of stored entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the store is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    // ---------------------------------------------------------------------
    // internals
    // ---------------------------------------------------------------------

    /// Rebuild the `by_id` map and the BM25 index from `entries`.
    fn rebuild(&mut self) {
        self.by_id.clear();
        self.index = Bm25Index::new();
        for (i, e) in self.entries.iter().enumerate() {
            self.by_id.insert(e.id.0.clone(), i);
            self.index.add(e.id.0.clone(), &tokenize(&doc_text(e)));
        }
    }

    /// Append a single entry as a JSON line.
    fn append_entry(&self, entry: &MemoryEntry) -> Result<()> {
        let line = serde_json::to_string(entry)?;
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|e| CoreError::from(e).with_context("opening memory.jsonl for append"))?;
        file.write_all(line.as_bytes())
            .and_then(|_| file.write_all(b"\n"))
            .map_err(|e| CoreError::from(e).with_context("appending memory entry"))?;
        Ok(())
    }

    /// Rewrite the whole file from the current entry set (used after in-place
    /// edits). Writes to a temp file then renames for atomicity.
    fn persist_all(&self) -> Result<()> {
        let tmp = self.path.with_extension("jsonl.tmp");
        {
            let mut file = fs::File::create(&tmp)
                .map_err(|e| CoreError::from(e).with_context("creating memory temp file"))?;
            for e in &self.entries {
                let line = serde_json::to_string(e)?;
                file.write_all(line.as_bytes())
                    .and_then(|_| file.write_all(b"\n"))
                    .map_err(|err| CoreError::from(err).with_context("writing memory temp file"))?;
            }
            file.flush()
                .map_err(|e| CoreError::from(e).with_context("flushing memory temp file"))?;
        }
        fs::rename(&tmp, &self.path)
            .map_err(|e| CoreError::from(e).with_context("replacing memory.jsonl"))?;
        Ok(())
    }
}

/// The text indexed for an entry: title + summary + tags + content. Combining
/// them means a query word found anywhere makes the entry recallable, while the
/// length normalization still favors focused entries.
fn doc_text(e: &MemoryEntry) -> String {
    let mut s = String::with_capacity(
        e.title.len()
            + e.summary.len()
            + e.content.len()
            + e.tags.iter().map(|t| t.len() + 1).sum::<usize>(),
    );
    s.push_str(&e.title);
    s.push(' ');
    s.push_str(&e.summary);
    s.push(' ');
    for t in &e.tags {
        s.push_str(t);
        s.push(' ');
    }
    s.push_str(&e.content);
    s
}

/// Load entries from a JSON-Lines file (missing file => empty). A blank line is
/// skipped; a malformed line aborts the load with a `Serialization` error so we
/// never silently lose memories.
fn load_entries(path: &Path) -> Result<Vec<MemoryEntry>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file = fs::File::open(path)
        .map_err(|e| CoreError::from(e).with_context("opening memory.jsonl"))?;
    let reader = BufReader::new(file);
    let mut out = Vec::new();
    for (lineno, line) in reader.lines().enumerate() {
        let line =
            line.map_err(|e| CoreError::from(e).with_context("reading memory.jsonl line"))?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let entry: MemoryEntry = serde_json::from_str(trimmed).map_err(|e| {
            CoreError::from(e).with_context(format!("parsing memory entry at line {}", lineno + 1))
        })?;
        out.push(entry);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct TempDir {
        path: PathBuf,
    }
    impl TempDir {
        fn new() -> Self {
            let mut p = std::env::temp_dir();
            p.push(format!("na_memory_store_{}", na_common::next_id("m")));
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

    // ---- tokenizer ----

    #[test]
    fn tokenize_english_words() {
        let t = tokenize("The Dragon-Lord, awakened!");
        assert_eq!(t, vec!["the", "dragon", "lord", "awakened"]);
    }

    #[test]
    fn tokenize_chinese_emits_unigrams_and_bigrams() {
        let t = tokenize("龙王");
        // unigrams 龙 王 then bigram 龙王
        assert!(t.contains(&"龙".to_string()));
        assert!(t.contains(&"王".to_string()));
        assert!(t.contains(&"龙王".to_string()));
        assert_eq!(t.len(), 3);
    }

    #[test]
    fn tokenize_mixed_and_lowercase() {
        let t = tokenize("Hero名叫Alice住在城堡");
        assert!(t.contains(&"hero".to_string()));
        assert!(t.contains(&"alice".to_string()));
        assert!(t.contains(&"名".to_string()));
        assert!(t.contains(&"名叫".to_string()));
        assert!(t.contains(&"城".to_string()));
        assert!(t.contains(&"城堡".to_string()));
    }

    #[test]
    fn chinese_query_matches_chinese_doc() {
        // This is the core CJK requirement.
        let mut idx = Bm25Index::new();
        idx.add("d1".into(), &tokenize("林惊羽是一名年轻的剑客，性格冷静。"));
        idx.add("d2".into(), &tokenize("城堡坐落在北方的雪山之上。"));
        idx.add("d3".into(), &tokenize("龙族统治着整个大陆。"));

        let hits = idx.search(&tokenize("剑客"), 5);
        assert!(
            !hits.is_empty(),
            "Chinese query must retrieve a Chinese doc"
        );
        assert_eq!(hits[0].0, "d1");

        let hits2 = idx.search(&tokenize("雪山"), 5);
        assert_eq!(hits2[0].0, "d2");
    }

    // ---- store ----

    #[test]
    fn save_recall_and_get() {
        let td = TempDir::new();
        let mut store = MemoryStore::open(td.file("memory.jsonl")).unwrap();

        let zh = store
            .save(
                MemoryKind::Character,
                "林惊羽",
                "冷静的年轻剑客，主角。",
                "林惊羽出身寒门，十六岁拜入青云门，使一柄名为‘霜寒’的长剑。",
                vec!["主角".into(), "剑客".into()],
                5,
            )
            .unwrap();
        let _en = store
            .save(
                MemoryKind::Setting,
                "Frost Keep",
                "A castle on the northern snow mountains.",
                "Frost Keep guards the only pass through the Wailing Range.",
                vec!["castle".into(), "north".into()],
                3,
            )
            .unwrap();

        // Chinese recall returns the structured header, not content.
        let hits = store.recall("剑客", 5, None, false);
        assert!(!hits.is_empty());
        assert_eq!(hits[0].id, zh);
        assert_eq!(hits[0].title, "林惊羽");
        // RecallHit has no `content` field at all — enforced at compile time.

        // English recall
        let hits_en = store.recall("castle snow", 5, None, false);
        assert_eq!(hits_en[0].title, "Frost Keep");

        // get() exposes full content on demand
        let full = store.get(&zh).unwrap();
        assert!(full.content.contains("霜寒"));
    }

    #[test]
    fn kind_filter_restricts_recall() {
        let td = TempDir::new();
        let mut store = MemoryStore::open(td.file("m.jsonl")).unwrap();
        store
            .save(
                MemoryKind::Character,
                "龙王",
                "古老的龙族之王",
                "龙王沉睡了千年。",
                vec![],
                4,
            )
            .unwrap();
        store
            .save(
                MemoryKind::Worldbuilding,
                "龙族",
                "统治大陆的龙族设定",
                "龙族掌控元素之力。",
                vec![],
                4,
            )
            .unwrap();

        // Without filter, querying "龙" should find both.
        let all = store.recall("龙", 10, None, false);
        assert!(all.len() >= 2);

        // With a Character filter, only the character entry survives.
        let only_char = store.recall("龙", 10, Some(MemoryKind::Character), false);
        assert!(only_char.iter().all(|h| h.kind == MemoryKind::Character));
        assert!(only_char.iter().any(|h| h.title == "龙王"));
        assert!(!only_char.iter().any(|h| h.title == "龙族"));
    }

    #[test]
    fn delete_removes_entry_and_reindexes() {
        let dir = TempDir::new();
        let path = dir.file("memory.jsonl");
        let mut store = MemoryStore::open(&path).unwrap();
        let id1 = store
            .save(MemoryKind::Character, "甲", "守门人", "正文一", vec![], 3)
            .unwrap();
        let id2 = store
            .save(MemoryKind::Character, "乙", "行脚商", "正文二", vec![], 3)
            .unwrap();
        assert_eq!(store.len(), 2);

        store.delete(&id1).unwrap();
        assert_eq!(store.len(), 1);
        assert!(store.get(&id1).is_none());
        assert!(store.get(&id2).is_some());
        // the surviving entry is still recallable (index rebuilt correctly)
        let hits = store.recall("行脚商", 5, None, false);
        assert!(hits.iter().any(|h| h.id == id2));

        // deletion persists across reopen
        let mut store2 = MemoryStore::open(&path).unwrap();
        assert_eq!(store2.len(), 1);
        assert!(store2.get(&id2).is_some());

        // deleting an unknown id is NotFound
        let err = store2.delete(&id1).unwrap_err();
        assert!(err.is(na_common::ErrorKind::NotFound));
    }

    #[test]
    fn archive_hides_from_recall_by_default() {
        let td = TempDir::new();
        let mut store = MemoryStore::open(td.file("m.jsonl")).unwrap();
        let id = store
            .save(
                MemoryKind::Plot,
                "废弃伏笔",
                "一条被弃用的伏笔",
                "这条线后来删掉了。",
                vec![],
                2,
            )
            .unwrap();

        assert_eq!(store.recall("伏笔", 5, None, false).len(), 1);
        store.archive(&id, true).unwrap();
        assert_eq!(
            store.recall("伏笔", 5, None, false).len(),
            0,
            "archived hidden by default"
        );
        // explicitly include archived
        assert_eq!(store.recall("伏笔", 5, None, true).len(), 1);
    }

    #[test]
    fn classify_updates_persist_and_survive_reopen() {
        let td = TempDir::new();
        let path = td.file("m.jsonl");
        let id;
        {
            let mut store = MemoryStore::open(&path).unwrap();
            id = store
                .save(
                    MemoryKind::Other,
                    "神秘符文",
                    "一段未知用途的符文",
                    "符文刻在剑柄上。",
                    vec![],
                    3,
                )
                .unwrap();
            store
                .classify(
                    &id,
                    Some(MemoryKind::Lore),
                    vec!["符文".into(), "符文".into()],
                )
                .unwrap();
        }
        // reopen from disk
        let store2 = MemoryStore::open(&path).unwrap();
        let e = store2.get(&id).unwrap();
        assert_eq!(e.kind, MemoryKind::Lore, "kind change persisted");
        assert_eq!(
            e.tags,
            vec!["符文".to_string()],
            "tags deduped and persisted"
        );

        // the new tag is searchable after reopen (index rebuilt from disk)
        let hits = store2.recall("符文", 5, None, false);
        assert!(hits.iter().any(|h| h.id == id));
    }

    #[test]
    fn archived_state_survives_reopen() {
        let td = TempDir::new();
        let path = td.file("m.jsonl");
        let id;
        {
            let mut store = MemoryStore::open(&path).unwrap();
            id = store
                .save(
                    MemoryKind::Dialogue,
                    "口头禅",
                    "角色的口头禅",
                    "‘有趣。’",
                    vec![],
                    1,
                )
                .unwrap();
            store.archive(&id, true).unwrap();
        }
        let store2 = MemoryStore::open(&path).unwrap();
        assert!(store2.get(&id).unwrap().archived);
        assert_eq!(store2.recall("口头禅", 5, None, false).len(), 0);
    }

    #[test]
    fn importance_is_clamped() {
        let td = TempDir::new();
        let mut store = MemoryStore::open(td.file("m.jsonl")).unwrap();
        let hi = store
            .save(MemoryKind::Other, "a", "s", "c", vec![], 99)
            .unwrap();
        let lo = store
            .save(MemoryKind::Other, "b", "s", "c", vec![], 0)
            .unwrap();
        assert_eq!(store.get(&hi).unwrap().importance, 5);
        assert_eq!(store.get(&lo).unwrap().importance, 1);
    }

    #[test]
    fn recall_ranks_more_relevant_first() {
        let td = TempDir::new();
        let mut store = MemoryStore::open(td.file("m.jsonl")).unwrap();
        store
            .save(
                MemoryKind::Character,
                "剑客甲",
                "一个普通剑客",
                "他是个剑客。",
                vec![],
                3,
            )
            .unwrap();
        let strong = store
            .save(
                MemoryKind::Character,
                "剑圣",
                "剑道宗师，剑客中的剑客",
                "剑圣以剑入道，剑客无人能及，号称剑客之巅。",
                vec!["剑客".into()],
                5,
            )
            .unwrap();
        let hits = store.recall("剑客", 5, None, false);
        assert_eq!(
            hits[0].id, strong,
            "the entry mentioning the term most ranks first"
        );
        // scores are populated and descending
        for w in hits.windows(2) {
            assert!(w[0].score >= w[1].score);
        }
    }

    #[test]
    fn classify_unknown_id_is_not_found() {
        let td = TempDir::new();
        let mut store = MemoryStore::open(td.file("m.jsonl")).unwrap();
        let err = store
            .classify(&MemoryId::from_existing("mem_missing"), None, vec![])
            .unwrap_err();
        assert!(err.is(na_common::ErrorKind::NotFound));
    }

    #[test]
    fn recall_zero_k_or_empty_query_returns_empty() {
        let td = TempDir::new();
        let mut store = MemoryStore::open(td.file("m.jsonl")).unwrap();
        store
            .save(MemoryKind::Other, "t", "s", "c", vec![], 1)
            .unwrap();
        assert!(store.recall("c", 0, None, false).is_empty());
        assert!(store.recall("   !!!  ", 5, None, false).is_empty());
    }

    #[test]
    fn bm25_retriever_trait_object_works() {
        // Exercise the object-safe Retriever trait via dyn.
        let mut r: Box<dyn Retriever> = Box::new(Bm25Retriever::new());
        r.index("a", "the quick brown fox");
        r.index("b", "龙王 沉睡");
        let en = r.retrieve("fox", 3);
        assert_eq!(en[0].0, "a");
        let zh = r.retrieve("龙王", 3);
        assert_eq!(zh[0].0, "b");
        r.clear();
        assert!(r.retrieve("fox", 3).is_empty());
    }

    #[test]
    fn entries_reload_in_order() {
        let td = TempDir::new();
        let path = td.file("m.jsonl");
        {
            let mut store = MemoryStore::open(&path).unwrap();
            store
                .save(MemoryKind::Other, "first", "s", "c", vec![], 1)
                .unwrap();
            store
                .save(MemoryKind::Other, "second", "s", "c", vec![], 1)
                .unwrap();
        }
        let store2 = MemoryStore::open(&path).unwrap();
        assert_eq!(store2.all().len(), 2);
        assert_eq!(store2.all()[0].title, "first");
        assert_eq!(store2.all()[1].title, "second");
    }
}
