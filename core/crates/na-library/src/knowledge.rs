//! Per-work knowledge bases — RAG corpora that keep creation on-setting.
//!
//! A single work can own several [`KnowledgeBase`]s. Each base is a directory
//! under the work's `knowledge/` dir:
//!
//! ```text
//! knowledge/
//! ├── <kb_id>/
//! │   ├── meta.json        # name, description, active flag, entry count
//! │   └── entries.jsonl    # one KnowledgeEntry per line
//! └── <kb_id_2>/ ...
//! ```
//!
//! Entries are indexed by the same CJK-aware BM25 retriever the long-term memory
//! uses (`na_memory::tokenize` + `na_memory::Bm25Index`), so [`search`] returns
//! the handful of canon facts relevant to a query. RAG injection searches across
//! every *active* base via [`KnowledgeStore::search_active`].

use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use na_common::time::now_millis;
use na_common::{next_id, CoreError, Result};
use na_memory::{tokenize, Bm25Index};
use serde::{Deserialize, Serialize};

/// Identifies one knowledge base within a work.
pub type KbId = String;

/// What a knowledge entry is *about* — lets the UI group and the agent filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeKind {
    /// A canon character (from the source material).
    Character,
    /// A place / location in the world.
    Location,
    /// A world rule, power system, faction, organization.
    Worldbuilding,
    /// A canon event / timeline fact.
    Event,
    /// An item, artifact, technique.
    Item,
    /// A term / glossary entry / proper noun.
    Term,
    /// General lore / trivia.
    Lore,
    /// Anything else.
    Other,
}

impl KnowledgeKind {
    pub fn all() -> &'static [KnowledgeKind] {
        use KnowledgeKind::*;
        &[
            Character,
            Location,
            Worldbuilding,
            Event,
            Item,
            Term,
            Lore,
            Other,
        ]
    }
}

/// One piece of canon knowledge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnowledgeEntry {
    pub id: String,
    pub kind: KnowledgeKind,
    /// Short label / heading (e.g. a name or term).
    pub title: String,
    /// The full canon fact.
    pub content: String,
    /// Where this came from: "user", "web:<url>", "memory", "ai".
    #[serde(default)]
    pub source: String,
    /// Free-form tags for filtering.
    #[serde(default)]
    pub tags: Vec<String>,
    pub created_ms: u64,
}

/// A retrieval hit: the entry plus its BM25 score and which base it came from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KnowledgeHit {
    pub entry: KnowledgeEntry,
    pub kb_id: KbId,
    pub kb_name: String,
    pub score: f32,
}

/// On-disk metadata for one knowledge base.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnowledgeBaseMeta {
    pub id: KbId,
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Whether this base participates in RAG retrieval for creation.
    #[serde(default = "default_true")]
    pub active: bool,
    pub created_ms: u64,
    pub updated_ms: u64,
    /// Cached entry count (kept in sync on mutation; for list views).
    #[serde(default)]
    pub entry_count: usize,
}

fn default_true() -> bool {
    true
}

/// A single knowledge base: its metadata + entries + a BM25 index.
pub struct KnowledgeBase {
    dir: PathBuf,
    pub meta: KnowledgeBaseMeta,
    entries: Vec<KnowledgeEntry>,
    by_id: HashMap<String, usize>,
    index: Bm25Index,
}

impl KnowledgeBase {
    fn meta_path(dir: &Path) -> PathBuf {
        dir.join("meta.json")
    }
    fn entries_path(dir: &Path) -> PathBuf {
        dir.join("entries.jsonl")
    }

    /// Open an existing base from its directory (expects a `meta.json`).
    fn open(dir: PathBuf) -> Result<Self> {
        let meta_text = fs::read_to_string(Self::meta_path(&dir))
            .map_err(|e| CoreError::from(e).with_context("reading kb meta"))?;
        let meta: KnowledgeBaseMeta = serde_json::from_str(&meta_text)
            .map_err(|e| CoreError::invalid_input(format!("corrupt kb meta: {e}")))?;
        let entries = load_entries(&Self::entries_path(&dir))?;
        let mut kb = KnowledgeBase {
            dir,
            meta,
            entries,
            by_id: HashMap::new(),
            index: Bm25Index::new(),
        };
        kb.rebuild();
        Ok(kb)
    }

    /// Create a fresh base directory with metadata.
    fn create(dir: PathBuf, name: impl Into<String>, description: impl Into<String>) -> Result<Self> {
        fs::create_dir_all(&dir)
            .map_err(|e| CoreError::from(e).with_context("creating kb directory"))?;
        let now = now_millis();
        let id = dir
            .file_name()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| next_id("kb"));
        let meta = KnowledgeBaseMeta {
            id,
            name: name.into(),
            description: description.into(),
            active: true,
            created_ms: now,
            updated_ms: now,
            entry_count: 0,
        };
        let mut kb = KnowledgeBase {
            dir,
            meta,
            entries: Vec::new(),
            by_id: HashMap::new(),
            index: Bm25Index::new(),
        };
        kb.save_meta()?;
        Ok(kb)
    }

    fn save_meta(&mut self) -> Result<()> {
        self.meta.entry_count = self.entries.len();
        self.meta.updated_ms = now_millis();
        let json = serde_json::to_string_pretty(&self.meta)?;
        let path = Self::meta_path(&self.dir);
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, json.as_bytes())
            .map_err(|e| CoreError::from(e).with_context("writing kb meta"))?;
        fs::rename(&tmp, &path)
            .map_err(|e| CoreError::from(e).with_context("replacing kb meta"))?;
        Ok(())
    }

    /// Rewrite the whole entries file atomically.
    fn save_entries(&self) -> Result<()> {
        let path = Self::entries_path(&self.dir);
        let tmp = path.with_extension("jsonl.tmp");
        let mut buf = String::new();
        for e in &self.entries {
            buf.push_str(&serde_json::to_string(e)?);
            buf.push('\n');
        }
        fs::write(&tmp, buf.as_bytes())
            .map_err(|e| CoreError::from(e).with_context("writing kb entries"))?;
        fs::rename(&tmp, &path)
            .map_err(|e| CoreError::from(e).with_context("replacing kb entries"))?;
        Ok(())
    }

    /// Rebuild the id map + BM25 index from the current entry set.
    fn rebuild(&mut self) {
        self.by_id.clear();
        self.index = Bm25Index::new();
        for (i, e) in self.entries.iter().enumerate() {
            self.by_id.insert(e.id.clone(), i);
            let doc = format!("{} {} {}", e.title, e.content, e.tags.join(" "));
            self.index.add(e.id.clone(), &tokenize(&doc));
        }
    }

    /// Add an entry and return its id.
    pub fn add(
        &mut self,
        kind: KnowledgeKind,
        title: impl Into<String>,
        content: impl Into<String>,
        source: impl Into<String>,
        tags: Vec<String>,
    ) -> Result<String> {
        let entry = KnowledgeEntry {
            id: next_id("ke"),
            kind,
            title: title.into(),
            content: content.into(),
            source: source.into(),
            tags,
            created_ms: now_millis(),
        };
        let id = entry.id.clone();
        self.entries.push(entry);
        self.save_entries()?;
        self.rebuild();
        self.save_meta()?;
        Ok(id)
    }

    /// Remove an entry by id (no-op if missing).
    pub fn remove(&mut self, entry_id: &str) -> Result<()> {
        if let Some(&i) = self.by_id.get(entry_id) {
            self.entries.remove(i);
            self.save_entries()?;
            self.rebuild();
            self.save_meta()?;
        }
        Ok(())
    }

    /// All entries (full content), newest first.
    pub fn entries(&self) -> Vec<KnowledgeEntry> {
        let mut v = self.entries.clone();
        v.sort_by(|a, b| b.created_ms.cmp(&a.created_ms));
        v
    }

    /// Top-`k` entries for `query` by BM25, best first.
    pub fn search(&self, query: &str, k: usize) -> Vec<(KnowledgeEntry, f32)> {
        let q = tokenize(query);
        self.index
            .search(&q, k)
            .into_iter()
            .filter_map(|(id, score)| {
                self.by_id
                    .get(&id)
                    .and_then(|&i| self.entries.get(i))
                    .map(|e| (e.clone(), score))
            })
            .collect()
    }
}

/// Load entries from a JSON-Lines file (missing file → empty, bad lines skipped).
fn load_entries(path: &Path) -> Result<Vec<KnowledgeEntry>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file = fs::File::open(path)
        .map_err(|e| CoreError::from(e).with_context("opening kb entries"))?;
    let reader = BufReader::new(file);
    let mut out = Vec::new();
    for line in reader.lines() {
        let line = line.map_err(|e| CoreError::from(e).with_context("reading kb entries"))?;
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        if let Ok(e) = serde_json::from_str::<KnowledgeEntry>(t) {
            out.push(e);
        }
    }
    Ok(out)
}

/// Manages all knowledge bases for one work (rooted at the work's `knowledge/`).
pub struct KnowledgeStore {
    root: PathBuf,
}

impl KnowledgeStore {
    /// Open (creating if needed) the knowledge directory for a work.
    pub fn open(knowledge_dir: impl AsRef<Path>) -> Result<Self> {
        let root = knowledge_dir.as_ref().to_path_buf();
        fs::create_dir_all(&root)
            .map_err(|e| CoreError::from(e).with_context("creating knowledge dir"))?;
        Ok(KnowledgeStore { root })
    }

    /// List the metadata of every base (most-recently-updated first).
    pub fn list_bases(&self) -> Result<Vec<KnowledgeBaseMeta>> {
        let mut out = Vec::new();
        let entries = match fs::read_dir(&self.root) {
            Ok(e) => e,
            Err(_) => return Ok(out),
        };
        for entry in entries.flatten() {
            let dir = entry.path();
            if !dir.is_dir() {
                continue;
            }
            let meta_path = dir.join("meta.json");
            if let Ok(text) = fs::read_to_string(&meta_path) {
                if let Ok(meta) = serde_json::from_str::<KnowledgeBaseMeta>(&text) {
                    out.push(meta);
                }
            }
        }
        out.sort_by(|a, b| b.updated_ms.cmp(&a.updated_ms));
        Ok(out)
    }

    /// Create a new base; returns its metadata.
    pub fn create_base(
        &self,
        name: impl Into<String>,
        description: impl Into<String>,
    ) -> Result<KnowledgeBaseMeta> {
        let id = next_id("kb");
        let dir = self.root.join(&id);
        let kb = KnowledgeBase::create(dir, name, description)?;
        Ok(kb.meta)
    }

    /// Open one base by id for reading / mutation.
    pub fn open_base(&self, kb_id: &str) -> Result<KnowledgeBase> {
        let dir = self.root.join(kb_id);
        if !dir.exists() {
            return Err(CoreError::not_found(format!("unknown knowledge base: {kb_id}")));
        }
        KnowledgeBase::open(dir)
    }

    /// Delete a base (its whole directory).
    pub fn delete_base(&self, kb_id: &str) -> Result<()> {
        let dir = self.root.join(kb_id);
        if dir.exists() {
            fs::remove_dir_all(&dir)
                .map_err(|e| CoreError::from(e).with_context("deleting knowledge base"))?;
        }
        Ok(())
    }

    /// Set a base's `active` flag (whether it participates in RAG).
    pub fn set_base_active(&self, kb_id: &str, active: bool) -> Result<KnowledgeBaseMeta> {
        let mut kb = self.open_base(kb_id)?;
        kb.meta.active = active;
        kb.save_meta()?;
        Ok(kb.meta)
    }

    /// Rename / re-describe a base.
    pub fn update_base(
        &self,
        kb_id: &str,
        name: Option<String>,
        description: Option<String>,
    ) -> Result<KnowledgeBaseMeta> {
        let mut kb = self.open_base(kb_id)?;
        if let Some(n) = name {
            kb.meta.name = n;
        }
        if let Some(d) = description {
            kb.meta.description = d;
        }
        kb.save_meta()?;
        Ok(kb.meta)
    }

    /// Search **across all active bases** and return the top-`k` hits globally
    /// (best first). This is what RAG injection uses before a creation run.
    pub fn search_active(&self, query: &str, k: usize) -> Result<Vec<KnowledgeHit>> {
        let mut hits: Vec<KnowledgeHit> = Vec::new();
        for meta in self.list_bases()? {
            if !meta.active {
                continue;
            }
            let kb = match self.open_base(&meta.id) {
                Ok(kb) => kb,
                Err(_) => continue,
            };
            for (entry, score) in kb.search(query, k) {
                hits.push(KnowledgeHit {
                    entry,
                    kb_id: meta.id.clone(),
                    kb_name: meta.name.clone(),
                    score,
                });
            }
        }
        hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        hits.truncate(k);
        Ok(hits)
    }

    /// The knowledge root directory.
    pub fn root(&self) -> &Path {
        &self.root
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> PathBuf {
        std::env::temp_dir().join(format!("na-kb-test-{}", next_id("t")))
    }

    #[test]
    fn create_add_search_roundtrip() {
        let root = tmp();
        let store = KnowledgeStore::open(&root).unwrap();
        let meta = store.create_base("斗破苍穹设定", "功法体系与人物").unwrap();
        let mut kb = store.open_base(&meta.id).unwrap();
        kb.add(
            KnowledgeKind::Worldbuilding,
            "斗气等级",
            "斗者、斗师、大斗师、斗灵、斗王、斗皇、斗宗、斗尊、斗圣、斗帝。",
            "user",
            vec!["功法".into()],
        )
        .unwrap();
        kb.add(
            KnowledgeKind::Character,
            "萧炎",
            "主角，废柴逆袭，吞噬异火，最终成为斗帝。",
            "user",
            vec!["主角".into()],
        )
        .unwrap();

        // reopen and search
        let kb2 = store.open_base(&meta.id).unwrap();
        let hits = kb2.search("斗帝 等级", 5);
        assert!(!hits.is_empty());

        // cross-base active search
        let active = store.search_active("萧炎", 3).unwrap();
        assert!(active.iter().any(|h| h.entry.title == "萧炎"));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn active_flag_filters_rag() {
        let root = tmp();
        let store = KnowledgeStore::open(&root).unwrap();
        let m = store.create_base("禁用库", "").unwrap();
        let mut kb = store.open_base(&m.id).unwrap();
        kb.add(KnowledgeKind::Lore, "秘辛", "不该被检索到的内容", "user", vec![])
            .unwrap();

        // active by default → found
        assert!(!store.search_active("秘辛", 5).unwrap().is_empty());
        // deactivate → excluded from RAG
        store.set_base_active(&m.id, false).unwrap();
        assert!(store.search_active("秘辛", 5).unwrap().is_empty());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn delete_entry_and_base() {
        let root = tmp();
        let store = KnowledgeStore::open(&root).unwrap();
        let m = store.create_base("临时库", "").unwrap();
        let mut kb = store.open_base(&m.id).unwrap();
        let id = kb
            .add(KnowledgeKind::Term, "术语", "一个术语", "user", vec![])
            .unwrap();
        assert_eq!(kb.entries().len(), 1);
        kb.remove(&id).unwrap();
        assert_eq!(kb.entries().len(), 0);

        store.delete_base(&m.id).unwrap();
        assert!(store.list_bases().unwrap().is_empty());

        let _ = fs::remove_dir_all(&root);
    }
}
