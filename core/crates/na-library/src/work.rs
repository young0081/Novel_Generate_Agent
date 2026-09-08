//! Multi-work management — a "book library" of isolated novel projects.
//!
//! Each [`WorkMeta`] is one novel: a title, a blurb, an optional genre, and the
//! path to its own private workspace directory. [`WorkStore`] keeps an index of
//! every work plus which one is active, all under a root library directory:
//!
//! ```text
//! <library_root>/
//! ├── works_index.json        # the list + the active id
//! └── works/
//!     ├── <work_id>/
//!     │   ├── workspace/       # manuscript (book/), memory, story_state, ...
//!     │   ├── sessions/        # this work's creation/discussion sessions
//!     │   └── knowledge/       # this work's knowledge bases
//!     └── <work_id_2>/ ...
//! ```
//!
//! On first run we adopt any pre-existing legacy `workspace/` (and `sessions/`)
//! sitting at the library root as a "默认作品", so an upgrading user keeps their
//! manuscript without a migration step.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use na_common::time::now_millis;
use na_common::{next_id, CoreError, Result};
use na_sandbox::PathJail;
use serde::{Deserialize, Serialize};

use crate::persist::atomic_write;

/// Identifies one work (novel project). A short, file-system-safe slug.
pub type WorkId = String;

/// Metadata + on-disk layout for a single novel project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkMeta {
    /// Stable id (also the directory name under `works/`).
    pub id: WorkId,
    /// Human title shown in the library.
    pub title: String,
    /// A one-to-few sentence description / logline.
    #[serde(default)]
    pub blurb: String,
    /// Optional genre tag (e.g. "玄幻", "同人", "都市").
    #[serde(default)]
    pub genre: String,
    /// Optional source material this is fan-fiction of (drives KB auto-fill).
    #[serde(default)]
    pub source_material: String,
    pub created_ms: u64,
    pub updated_ms: u64,
    /// Absolute path to this work's workspace (manuscript) directory.
    ///
    /// Stored explicitly so a legacy/default work can point at the old
    /// top-level `workspace/` while new works live under `works/<id>/workspace`.
    pub workspace_dir: PathBuf,
    /// Absolute path to this work's sessions directory.
    pub sessions_dir: PathBuf,
    /// Absolute path to this work's knowledge-bases directory.
    pub knowledge_dir: PathBuf,
}

impl WorkMeta {
    /// Build the standard layout for a brand-new work under `works/<id>/`.
    fn new_under(works_root: &Path, title: impl Into<String>) -> Self {
        let id = next_id("work");
        let base = works_root.join(&id);
        let now = now_millis();
        WorkMeta {
            id,
            title: title.into(),
            blurb: String::new(),
            genre: String::new(),
            source_material: String::new(),
            created_ms: now,
            updated_ms: now,
            workspace_dir: base.join("workspace"),
            sessions_dir: base.join("sessions"),
            knowledge_dir: base.join("knowledge"),
        }
    }
}

/// A lightweight summary for list views (avoids leaking absolute paths to the UI
/// when not needed, though they're harmless).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkSummary {
    pub id: WorkId,
    pub title: String,
    pub blurb: String,
    pub genre: String,
    pub source_material: String,
    pub created_ms: u64,
    pub updated_ms: u64,
    /// Whether this is the currently active work.
    pub active: bool,
}

/// The on-disk index document.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct IndexDoc {
    #[serde(default)]
    works: Vec<WorkMeta>,
    #[serde(default)]
    active: Option<WorkId>,
}

/// A directory-backed store of [`WorkMeta`]s plus the active selection.
#[derive(Debug, Clone)]
pub struct WorkStore {
    jail: PathJail,
    root: PathBuf,
    index_path: PathBuf,
    works_root: PathBuf,
    doc: IndexDoc,
}

impl WorkStore {
    /// Open (or initialize) the library rooted at `library_root`.
    ///
    /// If no index exists yet, one is created. When `legacy_workspace` is given
    /// and exists on disk, it's adopted as a "默认作品" pointing at that path (so
    /// an upgrading user keeps their existing manuscript). Otherwise the library
    /// simply starts empty (the GUI then prompts to create the first work).
    pub fn open(library_root: impl AsRef<Path>) -> Result<Self> {
        let jail = PathJail::new(library_root)?;
        let root = jail.root().to_path_buf();
        let works_root = root.join("works");
        fs::create_dir_all(&works_root)
            .map_err(|e| CoreError::from(e).with_context("creating works directory"))?;
        let index_path = root.join("works_index.json");
        let mut doc: IndexDoc = if index_path.exists() {
            let text = fs::read_to_string(&index_path)
                .map_err(|e| CoreError::from(e).with_context("reading works index"))?;
            serde_json::from_str(&text)
                .map_err(|e| CoreError::from(e).with_context("parsing works index"))?
        } else {
            IndexDoc::default()
        };
        let mut ids = HashSet::new();
        let mut managed_dirs: Vec<(String, &'static str, PathBuf)> = Vec::new();
        for work in &mut doc.works {
            validate_work_id(&work.id)?;
            if !ids.insert(work.id.clone()) {
                return Err(CoreError::invalid_input(format!(
                    "duplicate work id in index: {}",
                    work.id
                )));
            }
            work.workspace_dir = validate_library_path(&jail, &work.workspace_dir, "workspace")?;
            work.sessions_dir = validate_library_path(&jail, &work.sessions_dir, "sessions")?;
            work.knowledge_dir = validate_library_path(&jail, &work.knowledge_dir, "knowledge")?;
            for (label, path) in [
                ("workspace", &work.workspace_dir),
                ("sessions", &work.sessions_dir),
                ("knowledge", &work.knowledge_dir),
            ] {
                for (existing_id, existing_label, existing_path) in &managed_dirs {
                    if paths_overlap(path, existing_path) {
                        return Err(CoreError::invalid_input(format!(
                            "work {} {label} path overlaps work {existing_id} {existing_label}",
                            work.id
                        )));
                    }
                }
                managed_dirs.push((work.id.clone(), label, path.clone()));
            }
        }
        if let Some(active) = doc.active.as_deref() {
            validate_work_id(active)?;
            if !ids.contains(active) {
                return Err(CoreError::invalid_input(format!(
                    "active work id is not present in index: {active}"
                )));
            }
        }
        cleanup_committed_tombstones(&works_root, &ids);
        Ok(WorkStore {
            jail,
            root,
            index_path,
            works_root,
            doc,
        })
    }

    /// Adopt a pre-existing legacy workspace (+ optional sessions dir) as the
    /// default work IF the library is currently empty. No-op otherwise. Returns
    /// the adopted work's id when adoption happened.
    pub fn adopt_legacy(
        &mut self,
        legacy_workspace: &Path,
        legacy_sessions: &Path,
    ) -> Result<Option<WorkId>> {
        if !self.doc.works.is_empty() {
            return Ok(None);
        }
        if !legacy_workspace.exists() {
            return Ok(None);
        }
        let legacy_workspace =
            validate_library_path(&self.jail, legacy_workspace, "legacy workspace")?;
        let legacy_sessions =
            validate_library_path(&self.jail, legacy_sessions, "legacy sessions")?;
        let id = next_id("work");
        let now = now_millis();
        let knowledge_dir = self.works_root.join(&id).join("knowledge");
        let meta = WorkMeta {
            id: id.clone(),
            title: "默认作品".to_string(),
            blurb: "从旧版本自动收编的原有手稿".to_string(),
            genre: String::new(),
            source_material: String::new(),
            created_ms: now,
            updated_ms: now,
            workspace_dir: legacy_workspace,
            sessions_dir: legacy_sessions,
            knowledge_dir,
        };
        self.ensure_dirs(&meta)?;
        let mut next = self.doc.clone();
        next.works.push(meta);
        next.active = Some(id.clone());
        if let Err(error) = self.persist_doc(&next) {
            let _ = fs::remove_dir_all(self.works_root.join(&id));
            return Err(error);
        }
        self.doc = next;
        Ok(Some(id))
    }

    fn persist_doc(&self, doc: &IndexDoc) -> Result<()> {
        let json = serde_json::to_string_pretty(doc)?;
        atomic_write(&self.index_path, json.as_bytes(), "works index")
    }

    /// Create the on-disk directories for a work.
    fn ensure_dirs(&self, meta: &WorkMeta) -> Result<()> {
        for (label, d) in [
            ("workspace", &meta.workspace_dir),
            ("sessions", &meta.sessions_dir),
            ("knowledge", &meta.knowledge_dir),
        ] {
            let d = validate_library_path(&self.jail, d, label)?;
            fs::create_dir_all(d)
                .map_err(|e| CoreError::from(e).with_context("creating work directory"))?;
        }
        Ok(())
    }

    /// All works, newest-updated first, with the active flag set.
    pub fn list(&self) -> Vec<WorkSummary> {
        let active = self.doc.active.clone();
        let mut out: Vec<WorkSummary> = self
            .doc
            .works
            .iter()
            .map(|w| WorkSummary {
                id: w.id.clone(),
                title: w.title.clone(),
                blurb: w.blurb.clone(),
                genre: w.genre.clone(),
                source_material: w.source_material.clone(),
                created_ms: w.created_ms,
                updated_ms: w.updated_ms,
                active: active.as_deref() == Some(w.id.as_str()),
            })
            .collect();
        out.sort_by(|a, b| b.updated_ms.cmp(&a.updated_ms));
        out
    }

    /// The full metadata of one work.
    pub fn get(&self, id: &str) -> Option<&WorkMeta> {
        self.doc.works.iter().find(|w| w.id == id)
    }

    /// The active work's metadata, if any.
    pub fn active(&self) -> Option<&WorkMeta> {
        let id = self.doc.active.as_deref()?;
        self.get(id)
    }

    /// The active work's id, if any.
    pub fn active_id(&self) -> Option<&str> {
        self.doc.active.as_deref()
    }

    /// Create a new work, make its directories, and set it active. Returns it.
    pub fn create(
        &mut self,
        title: impl Into<String>,
        blurb: impl Into<String>,
        genre: impl Into<String>,
        source_material: impl Into<String>,
    ) -> Result<WorkMeta> {
        let mut meta = WorkMeta::new_under(&self.works_root, title);
        meta.blurb = blurb.into();
        meta.genre = genre.into();
        meta.source_material = source_material.into();
        self.ensure_dirs(&meta)?;
        let id = meta.id.clone();
        let mut next = self.doc.clone();
        next.works.push(meta.clone());
        next.active = Some(id.clone());
        if let Err(error) = self.persist_doc(&next) {
            let _ = fs::remove_dir_all(self.works_root.join(id));
            return Err(error);
        }
        self.doc = next;
        Ok(meta)
    }

    /// Initialize an independent work from a source, publishing it only after
    /// initialization succeeds. The callback must only write to the new work.
    pub fn create_from<T, F>(
        &mut self,
        source_id: &str,
        title: &str,
        initialize: F,
    ) -> Result<(WorkMeta, T)>
    where
        F: FnOnce(&WorkMeta, &WorkMeta) -> Result<T>,
    {
        validate_work_id(source_id)?;
        if title.trim().is_empty() {
            return Err(CoreError::invalid_input("新作品名称不能为空"));
        }
        let source = self
            .get(source_id)
            .cloned()
            .ok_or_else(|| CoreError::invalid_input("原作品不存在，请刷新书库"))?;
        let mut meta = WorkMeta::new_under(&self.works_root, title.trim());
        meta.blurb = source.blurb.clone();
        meta.genre = source.genre.clone();
        meta.source_material = source.source_material.clone();
        let base = self.works_root.join(&meta.id);
        // Claim a new directory exclusively: cleanup can never remove an
        // existing work, even if an ID collision occurs.
        fs::create_dir(&base)?;
        let result = (|| {
            self.ensure_dirs(&meta)?;
            let initialized = initialize(&source, &meta)?;
            let mut next = self.doc.clone();
            next.works.push(meta.clone());
            next.active = Some(meta.id.clone());
            self.persist_doc(&next)?;
            self.doc = next;
            Ok((meta, initialized))
        })();
        if result.is_err() {
            if let Err(cleanup) = fs::remove_dir_all(&base) {
                return result.map_err(|error: CoreError| {
                    error.with_context(format!(
                        "未完成的新作品目录清理失败 {}: {cleanup}",
                        base.display()
                    ))
                });
            }
        }
        result
    }

    /// Switch the active work. Errors if `id` is unknown.
    pub fn set_active(&mut self, id: &str) -> Result<()> {
        if !self.doc.works.iter().any(|w| w.id == id) {
            return Err(CoreError::invalid_input(format!("unknown work id: {id}")));
        }
        let mut next = self.doc.clone();
        next.active = Some(id.to_string());
        self.persist_doc(&next)?;
        self.doc = next;
        Ok(())
    }

    /// Rename / re-blurb / re-tag a work. Any `None` field is left unchanged.
    pub fn update(
        &mut self,
        id: &str,
        title: Option<String>,
        blurb: Option<String>,
        genre: Option<String>,
        source_material: Option<String>,
    ) -> Result<WorkMeta> {
        let mut next = self.doc.clone();
        let w = next
            .works
            .iter_mut()
            .find(|w| w.id == id)
            .ok_or_else(|| CoreError::invalid_input(format!("unknown work id: {id}")))?;
        if let Some(t) = title {
            w.title = t;
        }
        if let Some(b) = blurb {
            w.blurb = b;
        }
        if let Some(g) = genre {
            w.genre = g;
        }
        if let Some(s) = source_material {
            w.source_material = s;
        }
        w.updated_ms = now_millis();
        let out = w.clone();
        self.persist_doc(&next)?;
        self.doc = next;
        Ok(out)
    }

    /// Touch a work's `updated_ms` (called after a creation run so the library
    /// sorts recently-worked books to the top). Silent no-op if unknown.
    pub fn touch(&mut self, id: &str) -> Result<()> {
        let mut next = self.doc.clone();
        if let Some(w) = next.works.iter_mut().find(|w| w.id == id) {
            w.updated_ms = now_millis();
            self.persist_doc(&next)?;
            self.doc = next;
        }
        Ok(())
    }

    /// Delete a work from the index. When `purge_files` is true, its `works/<id>`
    /// directory tree is removed too (the legacy default work, whose workspace
    /// lives outside `works/`, only has its `works/<id>` subtree purged, never
    /// the adopted legacy workspace). The active selection falls back to the
    /// newest remaining work.
    pub fn delete(&mut self, id: &str, purge_files: bool) -> Result<()> {
        self.delete_with(id, purge_files, |dir| {
            fs::remove_dir_all(dir)
                .map_err(|error| CoreError::from(error).with_context("purging work directory"))
        })
    }

    fn delete_with<F>(&mut self, id: &str, purge_files: bool, purge: F) -> Result<()>
    where
        F: FnOnce(&Path) -> Result<()>,
    {
        validate_work_id(id)?;
        let idx = self
            .doc
            .works
            .iter()
            .position(|w| w.id == id)
            .ok_or_else(|| CoreError::invalid_input(format!("unknown work id: {id}")))?;
        let mut next = self.doc.clone();
        next.works.remove(idx);
        if next.active.as_deref() == Some(id) {
            // Fall back to the most-recently-updated remaining work.
            next.active = next
                .works
                .iter()
                .max_by_key(|work| work.updated_ms)
                .map(|work| work.id.clone());
        }

        let source = self.works_root.join(id);
        let tombstone = if purge_files && source.exists() {
            let tombstone =
                self.works_root
                    .join(format!(".deleted-{}-{id}-{}", id.len(), next_id("work")));
            fs::rename(&source, &tombstone).map_err(|error| {
                CoreError::from(error).with_context("staging work directory for deletion")
            })?;
            Some(tombstone)
        } else {
            None
        };

        if let Err(error) = self.persist_doc(&next) {
            if let Some(tombstone) = tombstone.as_ref() {
                if let Err(restore_error) = fs::rename(tombstone, &source) {
                    return Err(error.with_context(format!(
                        "also failed to restore staged work directory: {restore_error}"
                    )));
                }
            }
            return Err(error);
        }
        self.doc = next;

        if let Some(tombstone) = tombstone {
            // The logical deletion is already committed. Never restore a
            // potentially half-purged tombstone, but do report cleanup failure
            // so callers cannot claim that permanent deletion completed.
            purge(&tombstone).map_err(|error| {
                error.with_context(format!(
                    "work was removed from the library, but permanent cleanup remains at {}",
                    tombstone.display()
                ))
            })?;
        }
        Ok(())
    }

    /// The library root directory.
    pub fn root(&self) -> &Path {
        &self.root
    }
}

fn validate_work_id(id: &str) -> Result<()> {
    if id.is_empty()
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(CoreError::invalid_input(format!("invalid work id {id:?}")));
    }
    Ok(())
}

fn validate_library_path(jail: &PathJail, path: &Path, label: &str) -> Result<PathBuf> {
    if !path.is_absolute() {
        return Err(CoreError::invalid_input(format!(
            "work {label} path must be absolute: {}",
            path.display()
        )));
    }
    let requested = path.to_str().ok_or_else(|| {
        CoreError::invalid_input(format!("work {label} path is not valid Unicode"))
    })?;
    jail.resolve(requested).map_err(|error| {
        error.with_context(format!("validating work {label} path {}", path.display()))
    })
}

#[cfg(not(windows))]
fn paths_overlap(left: &Path, right: &Path) -> bool {
    let left = fs::canonicalize(left).unwrap_or_else(|_| left.to_path_buf());
    let right = fs::canonicalize(right).unwrap_or_else(|_| right.to_path_buf());
    left == right || left.starts_with(&right) || right.starts_with(&left)
}

#[cfg(windows)]
fn paths_overlap(left: &Path, right: &Path) -> bool {
    fn comparison_path(path: &Path) -> String {
        let path = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        path.to_string_lossy()
            .replace('/', "\\")
            .trim_end_matches('\\')
            .to_lowercase()
    }

    let left = comparison_path(left);
    let right = comparison_path(right);
    left == right
        || left
            .strip_prefix(&right)
            .is_some_and(|suffix| suffix.starts_with('\\'))
        || right
            .strip_prefix(&left)
            .is_some_and(|suffix| suffix.starts_with('\\'))
}

fn cleanup_committed_tombstones(works_root: &Path, live_ids: &HashSet<String>) {
    let Ok(entries) = fs::read_dir(works_root) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(work_id) = name.to_str().and_then(tombstone_work_id) else {
            continue;
        };
        if !live_ids.contains(work_id) {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

fn tombstone_work_id(name: &str) -> Option<&str> {
    let suffix = name.strip_prefix(".deleted-")?;
    let (length, suffix) = suffix.split_once('-')?;
    let length: usize = length.parse().ok()?;
    let id = suffix.get(..length)?;
    let remainder = suffix.get(length..)?;
    if remainder.starts_with('-') && validate_work_id(id).is_ok() {
        Some(id)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> PathBuf {
        std::env::temp_dir().join(format!("na-lib-test-{}", next_id("t")))
    }

    #[test]
    fn create_list_active_roundtrip() {
        let root = tmp();
        let mut store = WorkStore::open(&root).unwrap();
        assert!(store.list().is_empty());
        assert!(store.active().is_none());

        let a = store
            .create("斗破同人", "少年崛起", "同人", "斗破苍穹")
            .unwrap();
        assert!(a.workspace_dir.exists());
        assert!(a.knowledge_dir.exists());
        let b = store.create("都市修真", "", "都市", "").unwrap();

        // newest active
        assert_eq!(store.active_id(), Some(b.id.as_str()));
        let list = store.list();
        assert_eq!(list.len(), 2);
        // the active one is flagged
        assert!(list.iter().find(|w| w.id == b.id).unwrap().active);

        store.set_active(&a.id).unwrap();
        assert_eq!(store.active_id(), Some(a.id.as_str()));

        // reopen persists
        let store2 = WorkStore::open(&root).unwrap();
        assert_eq!(store2.active_id(), Some(a.id.as_str()));
        assert_eq!(store2.list().len(), 2);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn create_from_keeps_source_and_publishes_only_initialized_work() {
        let root = tempfile::tempdir().unwrap();
        let mut store = WorkStore::open(root.path()).unwrap();
        let source = store.create("原作", "简介", "修真", "原著").unwrap();
        let other = store.create("当前作品", "", "", "").unwrap();
        let (new_work, value) = store
            .create_from(&source.id, " 新作 ", |old, new| {
                assert_eq!(old, &source);
                assert!(new.sessions_dir.is_dir());
                fs::write(new.workspace_dir.join("outline.md"), "新作大纲")?;
                Ok(7)
            })
            .unwrap();
        assert_eq!(value, 7);
        assert_eq!(new_work.title, "新作");
        assert_eq!(new_work.blurb, source.blurb);
        assert_eq!(new_work.genre, source.genre);
        assert_eq!(new_work.source_material, source.source_material);
        assert_ne!(new_work.workspace_dir, source.workspace_dir);
        assert_eq!(store.get(&source.id), Some(&source));
        assert_eq!(store.get(&other.id), Some(&other));
        let reopened = WorkStore::open(root.path()).unwrap();
        assert_eq!(reopened.active_id(), Some(new_work.id.as_str()));
        assert_eq!(reopened.list().len(), 3);
    }

    #[test]
    fn create_from_rolls_back_copy_and_index_failures() {
        for fail_index in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let mut store = WorkStore::open(root.path()).unwrap();
            let source = store.create("原作", "", "", "").unwrap();
            let before = store.list();
            if fail_index {
                fs::remove_file(&store.index_path).unwrap();
                fs::create_dir(&store.index_path).unwrap();
            }
            let result = store.create_from(&source.id, "新作", |_, new| {
                fs::write(new.workspace_dir.join("partial.txt"), "partial")?;
                if fail_index {
                    Ok(())
                } else {
                    Err(CoreError::invalid_input("copy failed"))
                }
            });
            assert!(result.is_err());
            assert_eq!(store.list(), before);
            assert_eq!(store.active_id(), Some(source.id.as_str()));
            assert_eq!(fs::read_dir(&store.works_root).unwrap().count(), 1);
            if !fail_index {
                assert_eq!(WorkStore::open(root.path()).unwrap().list(), before);
            }
        }
    }

    #[test]
    fn create_from_rejects_unknown_source_and_blank_title() {
        let root = tempfile::tempdir().unwrap();
        let mut store = WorkStore::open(root.path()).unwrap();
        let source = store.create("原作", "", "", "").unwrap();
        for (id, title) in [
            ("../outside", "新作"),
            ("missing", "新作"),
            (source.id.as_str(), "  "),
        ] {
            assert!(store.create_from(id, title, |_, _| Ok(())).is_err());
        }
        assert_eq!(store.list().len(), 1);
        assert_eq!(fs::read_dir(&store.works_root).unwrap().count(), 1);
    }

    #[test]
    fn update_and_delete() {
        let root = tmp();
        let mut store = WorkStore::open(&root).unwrap();
        let a = store.create("旧名", "", "", "").unwrap();
        let b = store.create("第二部", "", "", "").unwrap();

        let updated = store
            .update(
                &a.id,
                Some("新名".into()),
                Some("新简介".into()),
                None,
                None,
            )
            .unwrap();
        assert_eq!(updated.title, "新名");
        assert_eq!(updated.blurb, "新简介");

        // delete active b → falls back to a
        store.set_active(&b.id).unwrap();
        store.delete(&b.id, true).unwrap();
        assert_eq!(store.active_id(), Some(a.id.as_str()));
        assert_eq!(store.list().len(), 1);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn partial_purge_never_restores_an_index_to_damaged_data() {
        let root = tmp();
        let mut store = WorkStore::open(&root).unwrap();
        let work = store.create("不能删除", "", "", "").unwrap();
        fs::write(work.workspace_dir.join("chapter.md"), "keep me").unwrap();

        let error = store
            .delete_with(&work.id, true, |tombstone| {
                fs::remove_file(tombstone.join("workspace/chapter.md")).unwrap();
                Err(CoreError::new(
                    na_common::ErrorKind::Io,
                    "injected partial purge failure",
                ))
            })
            .unwrap_err();
        assert!(error.to_string().contains("permanent cleanup remains"));
        assert!(store.active().is_none());
        assert!(store.get(&work.id).is_none());
        assert!(!work.workspace_dir.exists());
        assert!(fs::read_dir(&store.works_root).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".deleted-")
        }));

        let reopened = WorkStore::open(&root).unwrap();
        assert!(reopened.active().is_none());
        assert!(reopened.get(&work.id).is_none());
        assert_eq!(fs::read_dir(&reopened.works_root).unwrap().count(), 0);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn failed_delete_index_commit_restores_the_untouched_work_directory() {
        let root = tmp();
        let mut store = WorkStore::open(&root).unwrap();
        let work = store.create("保留作品", "", "", "").unwrap();
        fs::write(work.workspace_dir.join("chapter.md"), "keep me").unwrap();
        fs::remove_file(&store.index_path).unwrap();
        fs::create_dir(&store.index_path).unwrap();

        assert!(store.delete(&work.id, true).is_err());
        assert_eq!(store.active_id(), Some(work.id.as_str()));
        assert!(store.get(&work.id).is_some());
        assert_eq!(
            fs::read_to_string(work.workspace_dir.join("chapter.md")).unwrap(),
            "keep me"
        );
        assert!(!fs::read_dir(&store.works_root).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".deleted-")
        }));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn failed_create_keeps_memory_empty_and_removes_new_work_directory() {
        let root = tmp();
        let mut store = WorkStore::open(&root).unwrap();
        fs::create_dir(&store.index_path).unwrap();

        let error = store.create("不能提交", "", "", "").unwrap_err();
        assert!(error
            .context
            .as_deref()
            .is_some_and(|context| { context.contains("replacing works index") }));
        assert!(store.list().is_empty());
        assert!(store.active().is_none());
        assert_eq!(fs::read_dir(&store.works_root).unwrap().count(), 0);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn failed_index_updates_do_not_change_in_memory_state() {
        let root = tmp();
        let mut store = WorkStore::open(&root).unwrap();
        let first = store.create("第一部", "原简介", "", "").unwrap();
        let second = store.create("第二部", "", "", "").unwrap();
        store.set_active(&first.id).unwrap();
        let first_before = store.get(&first.id).unwrap().clone();

        fs::remove_file(&store.index_path).unwrap();
        fs::create_dir(&store.index_path).unwrap();

        assert!(store.set_active(&second.id).is_err());
        assert_eq!(store.active_id(), Some(first.id.as_str()));

        assert!(store
            .update(
                &first.id,
                Some("未提交的新名".into()),
                Some("未提交的新简介".into()),
                None,
                None,
            )
            .is_err());
        assert_eq!(store.get(&first.id), Some(&first_before));

        assert!(store.touch(&first.id).is_err());
        assert_eq!(store.get(&first.id), Some(&first_before));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn adopt_legacy_workspace() {
        let root = tmp();
        // simulate an existing legacy workspace with a file in it
        let legacy_ws = root.join("workspace");
        let legacy_sess = root.join("sessions");
        fs::create_dir_all(&legacy_ws).unwrap();
        fs::write(legacy_ws.join("ch1.md"), b"hello").unwrap();

        let mut store = WorkStore::open(&root).unwrap();
        let id = store.adopt_legacy(&legacy_ws, &legacy_sess).unwrap();
        assert!(id.is_some());
        let active = store.active().unwrap();
        assert_eq!(active.title, "默认作品");
        assert_eq!(active.workspace_dir, fs::canonicalize(&legacy_ws).unwrap());
        // adopting again is a no-op (library no longer empty)
        let again = store.adopt_legacy(&legacy_ws, &legacy_sess).unwrap();
        assert!(again.is_none());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn corrupt_index_is_reported_instead_of_silently_reset() {
        let root = tmp();
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("works_index.json"), "{not valid json").unwrap();

        let error = WorkStore::open(&root).unwrap_err();
        assert!(error.is(na_common::ErrorKind::Serialization), "{error}");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn index_cannot_redirect_work_paths_outside_the_library() {
        let root = tmp();
        let outside = tmp();
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        let now = now_millis();
        let malicious = serde_json::json!({
            "works": [{
                "id": "work_safe_id",
                "title": "malicious",
                "blurb": "",
                "genre": "",
                "source_material": "",
                "created_ms": now,
                "updated_ms": now,
                "workspace_dir": outside.join("workspace"),
                "sessions_dir": root.join("sessions"),
                "knowledge_dir": root.join("knowledge")
            }],
            "active": "work_safe_id"
        });
        fs::write(
            root.join("works_index.json"),
            serde_json::to_vec_pretty(&malicious).unwrap(),
        )
        .unwrap();

        let error = WorkStore::open(&root).unwrap_err();
        assert!(error.is(na_common::ErrorKind::SandboxViolation), "{error}");
        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(outside);
    }

    #[test]
    fn index_rejects_duplicate_ids_missing_active_work_and_overlapping_roots() {
        let cases = ["duplicate", "missing-active", "overlap"];
        for case in cases {
            let root = tmp();
            fs::create_dir_all(&root).unwrap();
            let works_root = root.join("works");
            let mut first = WorkMeta::new_under(&works_root, "第一部");
            let mut second = WorkMeta::new_under(&works_root, "第二部");
            let active = match case {
                "duplicate" => {
                    second.id = first.id.clone();
                    Some(first.id.clone())
                }
                "missing-active" => Some("work_missing".to_string()),
                "overlap" => {
                    second.workspace_dir = first.workspace_dir.clone();
                    Some(first.id.clone())
                }
                _ => unreachable!(),
            };
            first.workspace_dir = fs::canonicalize(&root).unwrap().join(
                first
                    .workspace_dir
                    .strip_prefix(&root)
                    .unwrap_or(&first.workspace_dir),
            );
            let doc = IndexDoc {
                works: vec![first, second],
                active,
            };
            fs::write(
                root.join("works_index.json"),
                serde_json::to_vec_pretty(&doc).unwrap(),
            )
            .unwrap();

            assert!(WorkStore::open(&root).is_err(), "case {case} was accepted");
            let _ = fs::remove_dir_all(root);
        }
    }
}
