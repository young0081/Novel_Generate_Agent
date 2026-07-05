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

use std::fs;
use std::path::{Path, PathBuf};

use na_common::time::now_millis;
use na_common::{next_id, CoreError, Result};
use serde::{Deserialize, Serialize};

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
        let root = library_root.as_ref().to_path_buf();
        fs::create_dir_all(&root)
            .map_err(|e| CoreError::from(e).with_context("creating library root"))?;
        let works_root = root.join("works");
        fs::create_dir_all(&works_root)
            .map_err(|e| CoreError::from(e).with_context("creating works directory"))?;
        let index_path = root.join("works_index.json");
        let doc = if index_path.exists() {
            let text = fs::read_to_string(&index_path)
                .map_err(|e| CoreError::from(e).with_context("reading works index"))?;
            serde_json::from_str(&text).unwrap_or_default()
        } else {
            IndexDoc::default()
        };
        Ok(WorkStore {
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
            workspace_dir: legacy_workspace.to_path_buf(),
            sessions_dir: legacy_sessions.to_path_buf(),
            knowledge_dir,
        };
        self.ensure_dirs(&meta)?;
        self.doc.works.push(meta);
        self.doc.active = Some(id.clone());
        self.save()?;
        Ok(Some(id))
    }

    /// Persist the index atomically.
    fn save(&self) -> Result<()> {
        let json = serde_json::to_string_pretty(&self.doc)?;
        let tmp = self.index_path.with_extension("json.tmp");
        fs::write(&tmp, json.as_bytes())
            .map_err(|e| CoreError::from(e).with_context("writing works index"))?;
        fs::rename(&tmp, &self.index_path)
            .map_err(|e| CoreError::from(e).with_context("replacing works index"))?;
        Ok(())
    }

    /// Create the on-disk directories for a work.
    fn ensure_dirs(&self, meta: &WorkMeta) -> Result<()> {
        for d in [&meta.workspace_dir, &meta.sessions_dir, &meta.knowledge_dir] {
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
        self.doc.works.push(meta.clone());
        self.doc.active = Some(id);
        self.save()?;
        Ok(meta)
    }

    /// Switch the active work. Errors if `id` is unknown.
    pub fn set_active(&mut self, id: &str) -> Result<()> {
        if !self.doc.works.iter().any(|w| w.id == id) {
            return Err(CoreError::invalid_input(format!("unknown work id: {id}")));
        }
        self.doc.active = Some(id.to_string());
        self.save()
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
        let w = self
            .doc
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
        self.save()?;
        Ok(out)
    }

    /// Touch a work's `updated_ms` (called after a creation run so the library
    /// sorts recently-worked books to the top). Silent no-op if unknown.
    pub fn touch(&mut self, id: &str) -> Result<()> {
        if let Some(w) = self.doc.works.iter_mut().find(|w| w.id == id) {
            w.updated_ms = now_millis();
            self.save()?;
        }
        Ok(())
    }

    /// Delete a work from the index. When `purge_files` is true, its `works/<id>`
    /// directory tree is removed too (the legacy default work, whose workspace
    /// lives outside `works/`, only has its `works/<id>` subtree purged, never
    /// the adopted legacy workspace). The active selection falls back to the
    /// newest remaining work.
    pub fn delete(&mut self, id: &str, purge_files: bool) -> Result<()> {
        let idx = self
            .doc
            .works
            .iter()
            .position(|w| w.id == id)
            .ok_or_else(|| CoreError::invalid_input(format!("unknown work id: {id}")))?;
        self.doc.works.remove(idx);
        if self.doc.active.as_deref() == Some(id) {
            // Fall back to the most-recently-updated remaining work.
            self.doc.active = self
                .list()
                .first()
                .map(|w| w.id.clone());
        }
        if purge_files {
            let dir = self.works_root.join(id);
            if dir.exists() {
                let _ = fs::remove_dir_all(&dir);
            }
        }
        self.save()
    }

    /// The library root directory.
    pub fn root(&self) -> &Path {
        &self.root
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

        let a = store.create("斗破同人", "少年崛起", "同人", "斗破苍穹").unwrap();
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
    fn update_and_delete() {
        let root = tmp();
        let mut store = WorkStore::open(&root).unwrap();
        let a = store.create("旧名", "", "", "").unwrap();
        let b = store.create("第二部", "", "", "").unwrap();

        let updated = store
            .update(&a.id, Some("新名".into()), Some("新简介".into()), None, None)
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
        assert_eq!(active.workspace_dir, legacy_ws);
        // adopting again is a no-op (library no longer empty)
        let again = store.adopt_legacy(&legacy_ws, &legacy_sess).unwrap();
        assert!(again.is_none());

        let _ = fs::remove_dir_all(&root);
    }
}
