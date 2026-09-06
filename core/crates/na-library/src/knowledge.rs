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
use std::collections::HashSet;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use na_common::time::now_millis;
use na_common::{next_id, CoreError, Result};
use na_memory::{tokenize, Bm25Index};
use serde::{Deserialize, Serialize};

use crate::persist::atomic_write;

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
            .map_err(|e| CoreError::from(e).with_context("parsing kb meta"))?;
        let entries = load_entries(&Self::entries_path(&dir))?;
        let mut entry_ids = HashSet::new();
        for entry in &entries {
            if entry.id.is_empty() {
                return Err(CoreError::invalid_input(
                    "knowledge base contains an entry with an empty id",
                ));
            }
            if !entry_ids.insert(entry.id.as_str()) {
                return Err(CoreError::invalid_input(format!(
                    "knowledge base contains duplicate entry id {:?}",
                    entry.id
                )));
            }
        }
        let directory_id = dir
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| CoreError::invalid_input("knowledge base directory has no valid id"))?;
        validate_kb_id(directory_id)?;
        if meta.id != directory_id {
            return Err(CoreError::invalid_input(format!(
                "knowledge base id mismatch: metadata has {:?}, directory is {directory_id:?}",
                meta.id
            )));
        }
        let mut kb = KnowledgeBase {
            dir,
            meta,
            entries,
            by_id: HashMap::new(),
            index: Bm25Index::new(),
        };
        kb.meta.entry_count = kb.entries.len();
        kb.rebuild();
        Ok(kb)
    }

    /// Create a fresh base directory with metadata.
    fn create(
        dir: PathBuf,
        name: impl Into<String>,
        description: impl Into<String>,
    ) -> Result<Self> {
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
        if let Err(error) = kb.save_meta() {
            let _ = fs::remove_dir_all(&kb.dir);
            return Err(error);
        }
        Ok(kb)
    }

    fn save_meta(&mut self) -> Result<()> {
        let mut next = self.meta.clone();
        next.entry_count = self.entries.len();
        next.updated_ms = now_millis();
        self.persist_meta(&next)?;
        self.meta = next;
        Ok(())
    }

    fn persist_meta(&self, meta: &KnowledgeBaseMeta) -> Result<()> {
        let json = serde_json::to_string_pretty(meta)?;
        atomic_write(
            &Self::meta_path(&self.dir),
            json.as_bytes(),
            "knowledge base metadata",
        )
    }

    fn persist_entries(&self, entries: &[KnowledgeEntry]) -> Result<()> {
        let mut buf = String::new();
        for e in entries {
            buf.push_str(&serde_json::to_string(e)?);
            buf.push('\n');
        }
        atomic_write(
            &Self::entries_path(&self.dir),
            buf.as_bytes(),
            "knowledge base entries",
        )
    }

    fn commit_entries(&mut self, next: Vec<KnowledgeEntry>) -> Result<()> {
        let previous = self.entries.clone();
        self.persist_entries(&next)?;

        let mut next_meta = self.meta.clone();
        next_meta.entry_count = next.len();
        next_meta.updated_ms = now_millis();
        if let Err(meta_error) = self.persist_meta(&next_meta) {
            if let Err(rollback_error) = self.persist_entries(&previous) {
                self.entries = next;
                self.meta = next_meta;
                self.rebuild();
                return Err(meta_error.with_context(format!(
                    "also failed to restore knowledge entries: {rollback_error}"
                )));
            }
            return Err(meta_error);
        }

        self.entries = next;
        self.meta = next_meta;
        self.rebuild();
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
        let mut next = self.entries.clone();
        next.push(entry);
        self.commit_entries(next)?;
        Ok(id)
    }

    /// Remove an entry by id (no-op if missing).
    pub fn remove(&mut self, entry_id: &str) -> Result<()> {
        if let Some(&i) = self.by_id.get(entry_id) {
            let mut next = self.entries.clone();
            next.remove(i);
            self.commit_entries(next)?;
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

/// Load entries from a JSON-Lines file (missing file => empty).
fn load_entries(path: &Path) -> Result<Vec<KnowledgeEntry>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file =
        fs::File::open(path).map_err(|e| CoreError::from(e).with_context("opening kb entries"))?;
    let reader = BufReader::new(file);
    let mut out = Vec::new();
    for (line_number, line) in reader.lines().enumerate() {
        let line = line.map_err(|e| CoreError::from(e).with_context("reading kb entries"))?;
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        let entry = serde_json::from_str::<KnowledgeEntry>(t).map_err(|error| {
            CoreError::from(error)
                .with_context(format!("parsing kb entries line {}", line_number + 1))
        })?;
        out.push(entry);
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
        let entries = fs::read_dir(&self.root)
            .map_err(|e| CoreError::from(e).with_context("reading knowledge directory"))?;
        for entry in entries {
            let entry = entry.map_err(|e| {
                CoreError::from(e).with_context("reading knowledge directory entry")
            })?;
            let dir = entry.path();
            if entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with(".deleted-"))
            {
                let _ = fs::remove_dir_all(&dir);
                continue;
            }
            let file_type = entry.file_type().map_err(|e| {
                CoreError::from(e).with_context(format!("reading type of {}", dir.display()))
            })?;
            if !file_type.is_dir() {
                continue;
            }
            let kb = KnowledgeBase::open(dir.clone()).map_err(|error| {
                error.with_context(format!("opening knowledge base {}", dir.display()))
            })?;
            out.push(kb.meta);
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
        validate_kb_id(kb_id)?;
        let dir = self.root.join(kb_id);
        if !dir.exists() {
            return Err(CoreError::not_found(format!(
                "unknown knowledge base: {kb_id}"
            )));
        }
        KnowledgeBase::open(dir)
    }

    /// Delete a base (its whole directory).
    pub fn delete_base(&self, kb_id: &str) -> Result<()> {
        self.delete_base_with(kb_id, |directory| {
            fs::remove_dir_all(directory)
                .map_err(|error| CoreError::from(error).with_context("purging knowledge base"))
        })
    }

    fn delete_base_with<F>(&self, kb_id: &str, purge: F) -> Result<()>
    where
        F: FnOnce(&Path) -> Result<()>,
    {
        validate_kb_id(kb_id)?;
        let dir = self.root.join(kb_id);
        if dir.exists() {
            let tombstone = self.root.join(format!(
                ".deleted-{}-{kb_id}-{}",
                kb_id.len(),
                next_id("kb")
            ));
            fs::rename(&dir, &tombstone).map_err(|error| {
                CoreError::from(error).with_context("staging knowledge base for deletion")
            })?;
            // Renaming commits the logical deletion. A recursive purge may
            // partially succeed before hitting a locked file, so never rename
            // a potentially damaged tombstone back into the live namespace.
            purge(&tombstone).map_err(|error| {
                error.with_context(format!(
                    "knowledge base was removed, but permanent cleanup remains at {}",
                    tombstone.display()
                ))
            })?;
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
            let kb = self.open_base(&meta.id).map_err(|error| {
                error.with_context(format!("searching knowledge base {}", meta.id))
            })?;
            for (entry, score) in kb.search(query, k) {
                hits.push(KnowledgeHit {
                    entry,
                    kb_id: meta.id.clone(),
                    kb_name: meta.name.clone(),
                    score,
                });
            }
        }
        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        hits.truncate(k);
        Ok(hits)
    }

    /// The knowledge root directory.
    pub fn root(&self) -> &Path {
        &self.root
    }
}

fn validate_kb_id(id: &str) -> Result<()> {
    if id.is_empty()
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(CoreError::invalid_input(format!(
            "invalid knowledge base id {id:?}"
        )));
    }
    Ok(())
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
        kb.add(
            KnowledgeKind::Lore,
            "秘辛",
            "不该被检索到的内容",
            "user",
            vec![],
        )
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

    #[test]
    fn partial_base_purge_never_reexposes_damaged_knowledge() {
        let root = tmp();
        let store = KnowledgeStore::open(&root).unwrap();
        let meta = store.create_base("待删除", "").unwrap();
        let mut kb = store.open_base(&meta.id).unwrap();
        kb.add(KnowledgeKind::Lore, "条目", "内容", "user", vec![])
            .unwrap();

        let error = store
            .delete_base_with(&meta.id, |tombstone| {
                fs::remove_file(tombstone.join("meta.json")).unwrap();
                Err(CoreError::new(
                    na_common::ErrorKind::Io,
                    "injected partial purge",
                ))
            })
            .unwrap_err();
        assert!(error.to_string().contains("permanent cleanup remains"));
        assert!(!root.join(&meta.id).exists());
        assert!(fs::read_dir(&root).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".deleted-")
        }));
        assert!(store.list_bases().unwrap().is_empty());
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn failed_entry_or_metadata_writes_keep_entries_and_index_coherent() {
        let entries_root = tmp();
        let entries_store = KnowledgeStore::open(&entries_root).unwrap();
        let entries_meta = entries_store.create_base("条目失败", "").unwrap();
        let mut entries_kb = entries_store.open_base(&entries_meta.id).unwrap();
        let existing_id = entries_kb
            .add(KnowledgeKind::Lore, "已有", "原内容", "user", vec![])
            .unwrap();
        let entries_path = entries_root.join(&entries_meta.id).join("entries.jsonl");
        fs::remove_file(&entries_path).unwrap();
        fs::create_dir(&entries_path).unwrap();

        assert!(entries_kb
            .add(KnowledgeKind::Lore, "未提交", "新内容", "user", vec![])
            .is_err());
        assert_eq!(entries_kb.entries().len(), 1);
        assert_eq!(entries_kb.entries()[0].id, existing_id);
        assert!(entries_kb
            .search("新内容", 5)
            .iter()
            .all(|(entry, _)| entry.title != "未提交"));
        assert_eq!(entries_kb.meta.entry_count, 1);

        let meta_root = tmp();
        let meta_store = KnowledgeStore::open(&meta_root).unwrap();
        let meta = meta_store.create_base("元数据失败", "").unwrap();
        let mut meta_kb = meta_store.open_base(&meta.id).unwrap();
        let kept_id = meta_kb
            .add(KnowledgeKind::Character, "保留", "不能丢", "user", vec![])
            .unwrap();
        let durable_entries = meta_root.join(&meta.id).join("entries.jsonl");
        let before = fs::read(&durable_entries).unwrap();
        let meta_path = meta_root.join(&meta.id).join("meta.json");
        fs::remove_file(&meta_path).unwrap();
        fs::create_dir(&meta_path).unwrap();

        assert!(meta_kb.remove(&kept_id).is_err());
        assert_eq!(meta_kb.entries().len(), 1);
        assert_eq!(meta_kb.entries()[0].id, kept_id);
        assert_eq!(fs::read(&durable_entries).unwrap(), before);
        assert_eq!(meta_kb.search("不能丢", 5).len(), 1);
        assert_eq!(meta_kb.meta.entry_count, 1);

        let _ = fs::remove_dir_all(entries_root);
        let _ = fs::remove_dir_all(meta_root);
    }

    #[test]
    fn knowledge_base_ids_cannot_escape_the_store() {
        let root = tmp();
        let store = KnowledgeStore::open(&root).unwrap();
        let outside = root
            .parent()
            .unwrap()
            .join(format!("outside-kb-{}", next_id("t")));
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("keep.txt"), "keep").unwrap();
        let attack = format!("../{}", outside.file_name().unwrap().to_string_lossy());

        assert!(store.open_base(&attack).is_err());
        assert!(store.delete_base(&attack).is_err());
        assert_eq!(
            fs::read_to_string(outside.join("keep.txt")).unwrap(),
            "keep"
        );
        let _ = fs::remove_dir_all(outside);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn malformed_entries_are_reported_and_preserved() {
        use std::io::Write;

        let root = tmp();
        let store = KnowledgeStore::open(&root).unwrap();
        let meta = store.create_base("损坏测试", "").unwrap();
        let mut kb = store.open_base(&meta.id).unwrap();
        kb.add(KnowledgeKind::Lore, "有效条目", "保留", "user", vec![])
            .unwrap();
        let entries_path = KnowledgeBase::entries_path(&root.join(&meta.id));
        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(&entries_path)
            .unwrap();
        file.write_all(b"{ malformed entry\n").unwrap();

        let error = match store.open_base(&meta.id) {
            Ok(_) => panic!("malformed entries should not open"),
            Err(error) => error,
        };
        assert!(error.is(na_common::ErrorKind::Serialization), "{error}");
        let raw = fs::read_to_string(&entries_path).unwrap();
        assert!(raw.contains("有效条目"));
        assert!(raw.contains("{ malformed entry"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn corrupt_metadata_is_reported_by_listing_and_rag_search() {
        let root = tmp();
        let store = KnowledgeStore::open(&root).unwrap();
        let meta = store.create_base("损坏元数据", "").unwrap();
        fs::write(root.join(&meta.id).join("meta.json"), "{ broken").unwrap();

        let list_error = store.list_bases().unwrap_err();
        assert!(
            list_error.is(na_common::ErrorKind::Serialization),
            "{list_error}"
        );
        let search_error = store.search_active("任意", 3).unwrap_err();
        assert!(
            search_error.is(na_common::ErrorKind::Serialization),
            "{search_error}"
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn listing_reports_an_unreadable_knowledge_root() {
        let root = tmp();
        let store = KnowledgeStore::open(&root).unwrap();
        fs::remove_dir(&root).unwrap();
        fs::write(&root, "not a directory").unwrap();

        let error = store.list_bases().unwrap_err();
        assert!(error.is(na_common::ErrorKind::Io), "{error}");

        let _ = fs::remove_file(root);
    }

    #[test]
    fn entry_count_is_recomputed_from_durable_entries() {
        let root = tmp();
        let store = KnowledgeStore::open(&root).unwrap();
        let meta = store.create_base("计数修复", "").unwrap();
        let mut kb = store.open_base(&meta.id).unwrap();
        kb.add(KnowledgeKind::Lore, "一条", "内容", "user", vec![])
            .unwrap();

        let meta_path = root.join(&meta.id).join("meta.json");
        let mut stale: KnowledgeBaseMeta =
            serde_json::from_str(&fs::read_to_string(&meta_path).unwrap()).unwrap();
        stale.entry_count = 99;
        fs::write(&meta_path, serde_json::to_vec_pretty(&stale).unwrap()).unwrap();

        assert_eq!(store.open_base(&meta.id).unwrap().meta.entry_count, 1);
        assert_eq!(store.list_bases().unwrap()[0].entry_count, 1);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn duplicate_entry_ids_are_rejected() {
        let root = tmp();
        let store = KnowledgeStore::open(&root).unwrap();
        let meta = store.create_base("重复标识", "").unwrap();
        let entry = KnowledgeEntry {
            id: "ke_duplicate".into(),
            kind: KnowledgeKind::Lore,
            title: "重复".into(),
            content: "内容".into(),
            source: "user".into(),
            tags: vec![],
            created_ms: now_millis(),
        };
        let raw = format!(
            "{}\n{}\n",
            serde_json::to_string(&entry).unwrap(),
            serde_json::to_string(&entry).unwrap()
        );
        fs::write(root.join(&meta.id).join("entries.jsonl"), raw).unwrap();

        assert!(store.open_base(&meta.id).is_err());

        let _ = fs::remove_dir_all(root);
    }
}
