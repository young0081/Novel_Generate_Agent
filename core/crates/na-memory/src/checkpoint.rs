//! Content-addressed workspace snapshots with undo/redo.
//!
//! A [`CheckpointStore`] lets the agent take a labelled snapshot of an entire
//! workspace directory before a risky edit, then [`restore`](CheckpointStore::restore)
//! it byte-for-byte (including deleting files that were created after the
//! snapshot). Snapshots are *content-addressed*: every distinct file body is
//! stored once under `objects/<hash>`, and a snapshot is just a manifest mapping
//! relative paths to those hashes. Identical files across many checkpoints share
//! a single blob, so snapshots are cheap.
//!
//! Manifests are persisted as JSON lines in `checkpoints.jsonl` so the store
//! survives a process restart (they are reloaded on [`open`](CheckpointStore::open)).
//!
//! The undo/redo stacks track *which checkpoint the workspace currently matches*.
//! `create` pushes the new id and clears redo; `undo` steps back to the previous
//! checkpoint's state; `redo` steps forward again.

use std::collections::{BTreeSet, HashSet};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Component, Path, PathBuf};

use na_common::time::now_millis;
use na_common::{CheckpointId, CoreError, Result};
use na_sandbox::PathJail;
use serde::{Deserialize, Serialize};

use crate::object_store::atomic_write_file;
use crate::{content_hash, read_content_object, validate_content_hash, write_content_object};

/// The persisted record of a single snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointManifest {
    pub id: CheckpointId,
    pub label: String,
    pub created_ms: u64,
    /// Sorted `(relative_path, content_hash)` pairs. Sorted for deterministic
    /// output and stable diffs.
    pub files: Vec<(String, String)>,
}

/// Lightweight metadata returned by [`CheckpointStore::list`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointMeta {
    pub id: CheckpointId,
    pub label: String,
    pub created_ms: u64,
    pub file_count: usize,
}

/// A content-addressed snapshot store rooted at a workspace directory.
pub struct CheckpointStore {
    workspace_root: PathBuf,
    store_dir: PathBuf,
    objects_dir: PathBuf,
    manifest_path: PathBuf,
    manifests: Vec<CheckpointManifest>,
    /// Stack of checkpoint ids whose states we have moved *into*; the last entry
    /// is the checkpoint the workspace currently matches.
    undo_stack: Vec<CheckpointId>,
    /// States we have stepped back from and can redo into.
    redo_stack: Vec<CheckpointId>,
}

impl CheckpointStore {
    /// Open (or initialize) a store. `store_dir` is created if missing, together
    /// with its `objects/` subdirectory. Existing manifests are reloaded so the
    /// list and history survive restarts. Note that the undo/redo stacks are
    /// in-memory only and start fresh on open (the on-disk snapshots remain
    /// fully usable via [`list`](Self::list) and [`restore`](Self::restore)).
    pub fn open(workspace_root: impl AsRef<Path>, store_dir: impl AsRef<Path>) -> Result<Self> {
        let jail = PathJail::new(workspace_root)?;
        let workspace_root = jail.root().to_path_buf();
        let store_dir = absolute_store_path(store_dir.as_ref())?;
        let objects_dir = store_dir.join("objects");
        let manifest_path = store_dir.join("checkpoints.jsonl");

        fs::create_dir_all(&workspace_root).map_err(|e| {
            CoreError::from(e).with_context("creating workspace root for checkpoint store")
        })?;
        fs::create_dir_all(&objects_dir)
            .map_err(|e| CoreError::from(e).with_context("creating checkpoint objects dir"))?;

        let manifests = load_manifests(&manifest_path)?;

        Ok(CheckpointStore {
            workspace_root,
            store_dir,
            objects_dir,
            manifest_path,
            manifests,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        })
    }

    /// Snapshot every file under the workspace root. Returns the new id and
    /// records it as the current state (clearing the redo stack).
    pub fn create(&mut self, label: &str) -> Result<CheckpointId> {
        let mut files: Vec<(String, String)> = Vec::new();
        let root = self.workspace_root.clone();
        self.snapshot_dir(&root, &mut files)?;
        files.sort();
        validate_manifest_ancestor_conflicts(&files)?;
        validate_windows_path_aliases(&files)?;

        let id = CheckpointId::new();
        let manifest = CheckpointManifest {
            id: id.clone(),
            label: label.to_string(),
            created_ms: now_millis(),
            files,
        };
        let mut next = self.manifests.clone();
        next.push(manifest);
        self.persist_manifest_set(&next)?;
        self.manifests = next;

        self.undo_stack.push(id.clone());
        self.redo_stack.clear();
        Ok(id)
    }

    /// Metadata for every known checkpoint, oldest first.
    pub fn list(&self) -> Vec<CheckpointMeta> {
        self.manifests
            .iter()
            .map(|m| CheckpointMeta {
                id: m.id.clone(),
                label: m.label.clone(),
                created_ms: m.created_ms,
                file_count: m.files.len(),
            })
            .collect()
    }

    /// Make the workspace exactly match snapshot `id`: (re)write every file in
    /// the manifest from its blob, and delete any file currently present that is
    /// not in the manifest. Empty directories left behind by deletions are pruned.
    pub fn restore(&mut self, id: &CheckpointId) -> Result<()> {
        let manifest = self
            .manifests
            .iter()
            .find(|m| &m.id == id)
            .cloned()
            .ok_or_else(|| CoreError::not_found(format!("checkpoint {id} not found")))?;
        self.apply_manifest(&manifest)
    }

    /// Step back to the state *before* the current checkpoint. Returns the id of
    /// the checkpoint the workspace now matches, or `None` if there is nothing to
    /// undo (fewer than two recorded states).
    ///
    /// The current state id is moved onto the redo stack so [`redo`](Self::redo)
    /// can return to it.
    pub fn undo(&mut self) -> Result<Option<CheckpointId>> {
        if self.undo_stack.len() < 2 {
            return Ok(None);
        }
        let current = self
            .undo_stack
            .pop()
            .ok_or_else(|| CoreError::internal("undo history changed unexpectedly"))?;
        let target = self
            .undo_stack
            .last()
            .cloned()
            .ok_or_else(|| CoreError::internal("undo history lost its target"))?;
        match self.restore_without_history(&target) {
            Ok(()) => {
                self.redo_stack.push(current);
                Ok(Some(target))
            }
            Err(error) => {
                self.undo_stack.push(current);
                Err(error)
            }
        }
    }

    /// Re-apply a state previously undone. Returns the id now matched, or `None`
    /// if there is nothing to redo.
    pub fn redo(&mut self) -> Result<Option<CheckpointId>> {
        let Some(target) = self.redo_stack.pop() else {
            return Ok(None);
        };
        match self.restore_without_history(&target) {
            Ok(()) => {
                self.undo_stack.push(target.clone());
                Ok(Some(target))
            }
            Err(error) => {
                self.redo_stack.push(target);
                Err(error)
            }
        }
    }

    /// The id the workspace currently matches according to the undo stack, if any.
    pub fn current(&self) -> Option<&CheckpointId> {
        self.undo_stack.last()
    }

    /// Permanently delete checkpoint `id`: drop its manifest, rewrite the
    /// manifest log, garbage-collect any blobs no longer referenced by a
    /// remaining checkpoint, and forget it in the undo/redo history. The
    /// workspace files themselves are untouched. Returns `NotFound` if unknown.
    pub fn delete(&mut self, id: &CheckpointId) -> Result<()> {
        let mut next = self.manifests.clone();
        next.retain(|m| &m.id != id);
        if next.len() == self.manifests.len() {
            return Err(CoreError::not_found(format!("checkpoint {id} not found")));
        }
        self.persist_manifest_set(&next)?;
        self.manifests = next;
        self.gc_objects()?;
        self.undo_stack.retain(|c| c != id);
        self.redo_stack.retain(|c| c != id);
        Ok(())
    }

    // ---------------------------------------------------------------------
    // internals
    // ---------------------------------------------------------------------

    /// Like [`restore`](Self::restore) but does not touch the undo/redo stacks
    /// (used by `undo`/`redo`, which manage the stacks themselves).
    fn restore_without_history(&mut self, id: &CheckpointId) -> Result<()> {
        let manifest = self
            .manifests
            .iter()
            .find(|m| &m.id == id)
            .cloned()
            .ok_or_else(|| CoreError::not_found(format!("checkpoint {id} not found")))?;
        self.apply_manifest(&manifest)
    }

    fn apply_manifest(&self, manifest: &CheckpointManifest) -> Result<()> {
        let plan = self.build_restore_plan(manifest)?;

        // Remove leaves first, then directories. Each path is reclassified
        // without following links immediately before it is changed.
        for entry in &plan.removals {
            self.remove_workspace_entry(entry)?;
        }

        for directory in &plan.directories {
            match fs::symlink_metadata(directory) {
                Ok(metadata) if metadata.file_type().is_dir() => {}
                Ok(_) => {
                    return Err(CoreError::conflict(format!(
                        "restore directory changed during apply: {}",
                        directory.display()
                    )))
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    fs::create_dir(directory).map_err(|error| {
                        CoreError::from(error).with_context(format!(
                            "creating restore directory {}",
                            directory.display()
                        ))
                    })?;
                }
                Err(error) => return Err(CoreError::from(error)),
            }
        }

        for write in plan.writes {
            // A leaf link that appears after preflight must not be followed or
            // atomically replaced. The helper repeats this check at rename time.
            atomic_write_file(
                &write.path,
                &write.bytes,
                &format!("restored workspace file {}", write.relative),
            )?;
        }
        Ok(())
    }

    fn build_restore_plan(&self, manifest: &CheckpointManifest) -> Result<RestorePlan> {
        let active_files: Vec<_> = manifest
            .files
            .iter()
            .filter(|(path, _)| classify_stored_path(path) == StoredPathClass::Active)
            .collect();
        let desired: BTreeSet<&str> = active_files.iter().map(|(path, _)| path.as_str()).collect();
        let desired_directories = desired_directories(&desired);

        let mut entries = Vec::new();
        self.collect_workspace_entries(&self.workspace_root, &mut entries)?;
        let mut removals = Vec::new();
        for entry in entries {
            let keep = match entry.kind {
                WorkspaceEntryKind::Directory => {
                    desired_directories.contains(entry.relative.as_str())
                        || self.directory_contains_protected_state(&entry.path)?
                }
                WorkspaceEntryKind::File => desired.contains(entry.relative.as_str()),
                WorkspaceEntryKind::LinkOrSpecial => false,
            };
            if !keep {
                removals.push(entry);
            }
        }
        removals.sort_by(|left, right| {
            path_depth(&right.relative)
                .cmp(&path_depth(&left.relative))
                .then_with(|| right.relative.cmp(&left.relative))
        });
        self.preflight_removals(&removals)?;

        // Refuse a plan whose required path is blocked by a link or by a
        // directory that cannot be replaced because it contains protected
        // state. This must precede every workspace mutation.
        for relative in desired.iter().chain(desired_directories.iter()) {
            self.preflight_required_path(relative)?;
        }

        let mut directories: Vec<PathBuf> = desired_directories
            .iter()
            .map(|path| lexical_workspace_path(&self.workspace_root, path))
            .collect();
        directories.sort_by_key(|path| path.components().count());

        let mut writes = Vec::with_capacity(active_files.len());
        for (relative, hash) in active_files {
            writes.push(RestoreWrite {
                relative: relative.clone(),
                path: lexical_workspace_path(&self.workspace_root, relative),
                bytes: self.read_blob(hash)?,
            });
        }

        Ok(RestorePlan {
            removals,
            directories,
            writes,
        })
    }

    fn preflight_removals(&self, removals: &[WorkspaceEntry]) -> Result<()> {
        let removal_paths: HashSet<&Path> =
            removals.iter().map(|entry| entry.path.as_path()).collect();
        for entry in removals
            .iter()
            .filter(|entry| entry.kind == WorkspaceEntryKind::Directory)
        {
            for child in fs::read_dir(&entry.path).map_err(|error| {
                CoreError::from(error)
                    .with_context(format!("preflighting removal of {}", entry.relative))
            })? {
                let child = child.map_err(CoreError::from)?;
                let child_path = child.path();
                if self.is_ignored(&child_path) || !removal_paths.contains(child_path.as_path()) {
                    return Err(CoreError::conflict(format!(
                        "restore would replace directory {:?} containing protected or untracked state",
                        entry.relative
                    )));
                }
            }
        }
        Ok(())
    }

    fn directory_contains_protected_state(&self, directory: &Path) -> Result<bool> {
        for entry in fs::read_dir(directory).map_err(|error| {
            CoreError::from(error).with_context(format!("inspecting {}", directory.display()))
        })? {
            let entry = entry.map_err(CoreError::from)?;
            let path = entry.path();
            if self.is_ignored(&path) {
                return Ok(true);
            }
            if entry.file_type().map_err(CoreError::from)?.is_dir()
                && self.directory_contains_protected_state(&path)?
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn preflight_required_path(&self, relative: &str) -> Result<()> {
        let mut current = self.workspace_root.clone();
        let components: Vec<&str> = relative.split('/').collect();
        for (index, component) in components.iter().enumerate() {
            current.push(component);
            let metadata = match fs::symlink_metadata(&current) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
                Err(error) => {
                    return Err(CoreError::from(error)
                        .with_context(format!("preflighting restore path {relative}")))
                }
            };
            if metadata.file_type().is_symlink() {
                return Err(CoreError::sandbox(format!(
                    "restore path {relative:?} contains a filesystem link"
                )));
            }
            if index + 1 == components.len()
                && metadata.file_type().is_dir()
                && self.directory_contains_protected_state(&current)?
            {
                return Err(CoreError::conflict(format!(
                    "restore path {relative:?} is blocked by protected state"
                )));
            }
        }
        Ok(())
    }

    fn remove_workspace_entry(&self, expected: &WorkspaceEntry) -> Result<()> {
        let metadata = match fs::symlink_metadata(&expected.path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(CoreError::from(error)),
        };
        let current_kind = WorkspaceEntryKind::from_metadata(&metadata);
        if current_kind != expected.kind {
            return Err(CoreError::conflict(format!(
                "workspace entry changed during restore: {}",
                expected.relative
            )));
        }
        let result = if metadata.file_type().is_dir() {
            fs::remove_dir(&expected.path)
        } else {
            fs::remove_file(&expected.path)
        };
        result.map_err(|error| {
            CoreError::from(error)
                .with_context(format!("removing {} during restore", expected.relative))
        })
    }

    /// Recursively walk `dir`, content-addressing every file and writing its blob
    /// if absent. Skips the store dir and any `.git` directory.
    fn snapshot_dir(&self, dir: &Path, out: &mut Vec<(String, String)>) -> Result<()> {
        let entries = fs::read_dir(dir).map_err(|e| {
            CoreError::from(e).with_context(format!("reading dir {}", dir.display()))
        })?;
        for entry in entries {
            let entry = entry.map_err(CoreError::from)?;
            let path = entry.path();
            if self.is_ignored(&path) {
                continue;
            }
            let file_type = entry.file_type().map_err(CoreError::from)?;
            if file_type.is_dir() {
                self.snapshot_dir(&path, out)?;
            } else if file_type.is_file() {
                let rel = self.rel_path(&path)?;
                if classify_stored_path(&rel) != StoredPathClass::Active {
                    return Err(invalid_stored_path(&rel));
                }
                let bytes = fs::read(&path).map_err(|e| {
                    CoreError::from(e).with_context(format!("reading file {}", path.display()))
                })?;
                let hash = content_hash(&bytes);
                self.write_blob_if_absent(&hash, &bytes)?;
                out.push((rel, hash));
            }
            // symlinks and other special files are skipped intentionally.
        }
        Ok(())
    }

    /// Collect every non-protected entry without following filesystem links.
    fn collect_workspace_entries(&self, dir: &Path, out: &mut Vec<WorkspaceEntry>) -> Result<()> {
        let entries = fs::read_dir(dir).map_err(|e| {
            CoreError::from(e).with_context(format!("reading dir {}", dir.display()))
        })?;
        for entry in entries {
            let entry = entry.map_err(CoreError::from)?;
            let path = entry.path();
            if self.is_ignored(&path) {
                continue;
            }
            let file_type = entry.file_type().map_err(CoreError::from)?;
            let relative = self.rel_path(&path)?;
            if file_type.is_dir() {
                out.push(WorkspaceEntry {
                    relative,
                    path: path.clone(),
                    kind: WorkspaceEntryKind::Directory,
                });
                self.collect_workspace_entries(&path, out)?;
            } else {
                out.push(WorkspaceEntry {
                    relative,
                    path,
                    kind: if file_type.is_file() {
                        WorkspaceEntryKind::File
                    } else {
                        WorkspaceEntryKind::LinkOrSpecial
                    },
                });
            }
        }
        Ok(())
    }

    /// True if `path` is the store dir, inside it, or a `.git` directory.
    fn is_ignored(&self, path: &Path) -> bool {
        // Use lexical containment here. Canonicalizing an arbitrary workspace
        // entry would follow a symlink and could incorrectly classify the link
        // itself as protected internal state.
        if path == self.store_dir || path.starts_with(&self.store_dir) {
            return true;
        }
        path.file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| {
                name.eq_ignore_ascii_case(".na")
                    || name.eq_ignore_ascii_case(".na-vcs")
                    || name.eq_ignore_ascii_case(".git")
            })
    }

    /// Relative path from the workspace root, normalized to forward slashes.
    fn rel_path(&self, path: &Path) -> Result<String> {
        let rel = path.strip_prefix(&self.workspace_root).map_err(|_| {
            CoreError::internal(format!(
                "path {} is not under workspace root {}",
                path.display(),
                self.workspace_root.display()
            ))
        })?;
        let mut parts: Vec<String> = Vec::new();
        for comp in rel.components() {
            match comp {
                Component::Normal(os) => {
                    let part = os.to_str().ok_or_else(|| {
                        CoreError::invalid_input(format!(
                            "checkpoint cannot represent non-UTF-8 file name in {}",
                            path.display()
                        ))
                    })?;
                    parts.push(part.to_owned());
                }
                // workspace-relative paths should never contain these, but be safe.
                Component::CurDir => {}
                _ => {
                    return Err(CoreError::internal(format!(
                        "unexpected path component in {}",
                        rel.display()
                    )))
                }
            }
        }
        Ok(parts.join("/"))
    }

    fn write_blob_if_absent(&self, hash: &str, bytes: &[u8]) -> Result<()> {
        write_content_object(&self.objects_dir, hash, bytes)
    }

    fn read_blob(&self, hash: &str) -> Result<Vec<u8>> {
        read_content_object(&self.objects_dir, hash)
    }

    /// Rewrite the complete log with one atomic replacement. This avoids a torn
    /// final JSON line if a checkpoint creation is interrupted.
    fn persist_manifest_set(&self, manifests: &[CheckpointManifest]) -> Result<()> {
        let mut bytes = Vec::new();
        for manifest in manifests {
            serde_json::to_writer(&mut bytes, manifest)?;
            bytes.push(b'\n');
        }
        atomic_write_file(&self.manifest_path, &bytes, "checkpoint manifest log")
    }

    /// Remove blob objects no longer referenced by any remaining manifest.
    /// Best-effort: a blob we cannot remove is left in place (no error).
    fn gc_objects(&self) -> Result<()> {
        use std::collections::HashSet;
        let referenced: HashSet<&str> = self
            .manifests
            .iter()
            .flat_map(|m| m.files.iter().map(|(_, hash)| hash.as_str()))
            .collect();
        let read = match fs::read_dir(&self.objects_dir) {
            Ok(r) => r,
            Err(_) => return Ok(()),
        };
        for entry in read.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if !referenced.contains(name) {
                    let _ = fs::remove_file(&path);
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
struct RestorePlan {
    removals: Vec<WorkspaceEntry>,
    directories: Vec<PathBuf>,
    writes: Vec<RestoreWrite>,
}

#[derive(Debug)]
struct RestoreWrite {
    relative: String,
    path: PathBuf,
    bytes: Vec<u8>,
}

#[derive(Debug)]
struct WorkspaceEntry {
    relative: String,
    path: PathBuf,
    kind: WorkspaceEntryKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkspaceEntryKind {
    File,
    Directory,
    LinkOrSpecial,
}

impl WorkspaceEntryKind {
    fn from_metadata(metadata: &fs::Metadata) -> Self {
        if metadata.file_type().is_dir() {
            Self::Directory
        } else if metadata.file_type().is_file() {
            Self::File
        } else {
            Self::LinkOrSpecial
        }
    }
}

fn desired_directories<'a>(files: &BTreeSet<&'a str>) -> BTreeSet<&'a str> {
    let mut directories = BTreeSet::new();
    for file in files {
        let mut end = 0;
        while let Some(offset) = file[end..].find('/') {
            end += offset;
            directories.insert(&file[..end]);
            end += 1;
        }
    }
    directories
}

fn lexical_workspace_path(root: &Path, relative: &str) -> PathBuf {
    let mut path = root.to_path_buf();
    path.extend(relative.split('/'));
    path
}

fn path_depth(relative: &str) -> usize {
    relative.bytes().filter(|byte| *byte == b'/').count()
}

fn absolute_store_path(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    std::env::current_dir()
        .map(|current| current.join(path))
        .map_err(|error| CoreError::from(error).with_context("resolving checkpoint store path"))
}

/// Read and validate all manifests from `path` (missing file => empty vec).
fn load_manifests(path: &Path) -> Result<Vec<CheckpointManifest>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.file_type().is_file() => {
            return Err(CoreError::sandbox(
                "checkpoint manifest is not a regular file",
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(CoreError::from(error).with_context("reading checkpoints.jsonl metadata"));
        }
    }
    let file = fs::File::open(path)
        .map_err(|e| CoreError::from(e).with_context("opening checkpoints.jsonl"))?;
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(CoreError::sandbox(
                "checkpoint manifest changed to a filesystem link while opening",
            ));
        }
        Ok(_) => {}
        Err(error) => return Err(CoreError::from(error)),
    }
    let opened = file
        .metadata()
        .map_err(|e| CoreError::from(e).with_context("inspecting open checkpoints.jsonl"))?;
    if !opened.file_type().is_file() {
        return Err(CoreError::sandbox(
            "checkpoint manifest changed while it was opened",
        ));
    }
    let reader = BufReader::new(file);
    let mut out = Vec::new();
    let mut ids = HashSet::new();
    for (lineno, line) in reader.lines().enumerate() {
        let line = line.map_err(|e| {
            CoreError::from(e)
                .with_context(format!("reading checkpoints.jsonl line {}", lineno + 1))
        })?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let manifest: CheckpointManifest = serde_json::from_str(line).map_err(|e| {
            CoreError::from(e).with_context(format!(
                "parsing checkpoint manifest at line {}",
                lineno + 1
            ))
        })?;
        if manifest.id.0.is_empty() || !ids.insert(manifest.id.0.clone()) {
            return Err(CoreError::new(
                na_common::ErrorKind::Serialization,
                format!("invalid or duplicate checkpoint id at line {}", lineno + 1),
            ));
        }
        let mut paths = HashSet::new();
        for (relative_path, hash) in &manifest.files {
            if classify_stored_path(relative_path) == StoredPathClass::Invalid {
                return Err(invalid_stored_path(relative_path)
                    .with_context(format!("checkpoint manifest line {}", lineno + 1)));
            }
            if !paths.insert(relative_path.as_str()) {
                return Err(CoreError::new(
                    na_common::ErrorKind::Serialization,
                    format!(
                        "duplicate checkpoint path {relative_path:?} at line {}",
                        lineno + 1
                    ),
                ));
            }
            validate_content_hash(hash).map_err(|error| {
                error.with_context(format!("checkpoint manifest line {}", lineno + 1))
            })?;
        }
        validate_manifest_ancestor_conflicts(&manifest.files).map_err(|error| {
            error.with_context(format!("checkpoint manifest line {}", lineno + 1))
        })?;
        validate_windows_path_aliases(&manifest.files).map_err(|error| {
            error.with_context(format!("checkpoint manifest line {}", lineno + 1))
        })?;
        out.push(manifest);
    }
    out.sort_by_key(|m| m.created_ms);
    Ok(out)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StoredPathClass {
    Active,
    ProtectedLegacy,
    Invalid,
}

fn classify_stored_path(path: &str) -> StoredPathClass {
    if path.is_empty() || path.contains('\0') {
        return StoredPathClass::Invalid;
    }
    let mut protected = false;
    for part in path.split('/') {
        if part.is_empty()
            || part == "."
            || part == ".."
            || part.contains(':')
            || part.contains('\\')
        {
            return StoredPathClass::Invalid;
        }
        if is_reserved_component(part) {
            protected = true;
        }
    }
    if protected {
        StoredPathClass::ProtectedLegacy
    } else {
        StoredPathClass::Active
    }
}

fn is_reserved_component(component: &str) -> bool {
    component.eq_ignore_ascii_case(".na")
        || component.eq_ignore_ascii_case(".na-vcs")
        || component.eq_ignore_ascii_case(".git")
}

fn validate_manifest_ancestor_conflicts(files: &[(String, String)]) -> Result<()> {
    let active: HashSet<&str> = files
        .iter()
        .filter_map(|(path, _)| {
            (classify_stored_path(path) == StoredPathClass::Active).then_some(path.as_str())
        })
        .collect();
    for path in &active {
        let mut end = 0;
        while let Some(offset) = path[end..].find('/') {
            end += offset;
            if active.contains(&path[..end]) {
                return Err(CoreError::new(
                    na_common::ErrorKind::Serialization,
                    format!("checkpoint paths {:?} and {path:?} conflict", &path[..end]),
                ));
            }
            end += 1;
        }
    }
    Ok(())
}

#[cfg(windows)]
fn windows_path_key(path: &str) -> String {
    path.split('/')
        .map(|component| component.trim_end_matches([' ', '.']).to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(windows)]
fn validate_windows_path_aliases(files: &[(String, String)]) -> Result<()> {
    let mut keys = HashSet::new();
    for (path, _) in files
        .iter()
        .filter(|(path, _)| classify_stored_path(path) == StoredPathClass::Active)
    {
        let key = windows_path_key(path);
        let has_trimmed_component = path
            .split('/')
            .any(|component| component.trim_end_matches([' ', '.']) != component);
        if has_trimmed_component
            || key.split('/').any(|component| {
                let stem = component.split('.').next().unwrap_or(component);
                matches!(
                    stem,
                    "con"
                        | "prn"
                        | "aux"
                        | "nul"
                        | "com1"
                        | "com2"
                        | "com3"
                        | "com4"
                        | "com5"
                        | "com6"
                        | "com7"
                        | "com8"
                        | "com9"
                        | "lpt1"
                        | "lpt2"
                        | "lpt3"
                        | "lpt4"
                        | "lpt5"
                        | "lpt6"
                        | "lpt7"
                        | "lpt8"
                        | "lpt9"
                )
            })
            || !keys.insert(key)
        {
            return Err(invalid_stored_path(path));
        }
    }
    Ok(())
}

#[cfg(not(windows))]
fn validate_windows_path_aliases(_files: &[(String, String)]) -> Result<()> {
    Ok(())
}

fn invalid_stored_path(path: &str) -> CoreError {
    CoreError::new(
        na_common::ErrorKind::Serialization,
        format!("invalid checkpoint path {path:?}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// A throwaway temp dir under the OS temp dir, removed on drop.
    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new(tag: &str) -> Self {
            let mut p = std::env::temp_dir();
            let unique = na_common::next_id(tag);
            p.push(format!("na_memory_test_{unique}"));
            fs::create_dir_all(&p).unwrap();
            TempDir { path: p }
        }
        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    /// Snapshot the on-disk workspace as a map of rel-path -> bytes, ignoring the
    /// store dir, so we can assert byte-for-byte equality after restore.
    fn read_workspace(root: &Path, store: &Path) -> BTreeMap<String, Vec<u8>> {
        let mut out = BTreeMap::new();
        fn walk(dir: &Path, root: &Path, store: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
            for entry in fs::read_dir(dir).unwrap().flatten() {
                let path = entry.path();
                if path.starts_with(store)
                    || path.file_name().and_then(|n| n.to_str()) == Some(".git")
                {
                    continue;
                }
                if path.is_dir() {
                    walk(&path, root, store, out);
                } else if path.is_file() {
                    let rel = path
                        .strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/");
                    out.insert(rel, fs::read(&path).unwrap());
                }
            }
        }
        walk(root, root, store, &mut out);
        out
    }

    #[test]
    fn delete_removes_checkpoint_gcs_blobs_and_persists() {
        let ws = TempDir::new("ws");
        let st = TempDir::new("st");
        let root = ws.path();
        let mut cs = CheckpointStore::open(root, st.path()).unwrap();

        fs::write(root.join("a.txt"), b"hello").unwrap();
        let id = cs.create("snap").unwrap();
        assert_eq!(cs.list().len(), 1);

        let objects = st.path().join("objects");
        assert!(fs::read_dir(&objects).unwrap().count() >= 1);

        cs.delete(&id).unwrap();
        assert_eq!(cs.list().len(), 0);
        // its sole blob is now unreferenced → garbage-collected
        assert_eq!(fs::read_dir(&objects).unwrap().count(), 0);
        assert!(cs.current().is_none());

        // deletion persists across reopen
        let cs2 = CheckpointStore::open(root, st.path()).unwrap();
        assert_eq!(cs2.list().len(), 0);

        // deleting an unknown id is NotFound
        let err = cs.delete(&id).unwrap_err();
        assert!(err.is(na_common::ErrorKind::NotFound));
    }

    #[test]
    fn delete_keeps_blobs_shared_with_other_checkpoints() {
        let ws = TempDir::new("ws2");
        let st = TempDir::new("st2");
        let root = ws.path();
        let mut cs = CheckpointStore::open(root, st.path()).unwrap();

        // same content in two checkpoints → one shared blob
        fs::write(root.join("a.txt"), b"shared").unwrap();
        let c1 = cs.create("one").unwrap();
        fs::write(root.join("b.txt"), b"shared").unwrap();
        let _c2 = cs.create("two").unwrap();

        cs.delete(&c1).unwrap();
        assert_eq!(cs.list().len(), 1);
        // the shared blob is still referenced by c2, so it survives GC
        let objects = st.path().join("objects");
        assert_eq!(fs::read_dir(&objects).unwrap().count(), 1);
    }

    #[test]
    fn content_hash_is_stable_and_length_tagged() {
        let a = content_hash(b"hello");
        let b = content_hash(b"hello");
        assert_eq!(a, b);
        assert!(a.starts_with("5-"), "len 5 prefix expected, got {a}");
        assert_ne!(content_hash(b"hello"), content_hash(b"world"));
        // empty input is handled
        assert!(content_hash(b"").starts_with("0-"));
    }

    #[test]
    fn snapshot_restore_is_byte_identical() {
        let ws = TempDir::new("ws");
        let st = TempDir::new("store");
        let root = ws.path();
        let store = st.path();

        // initial workspace
        fs::write(root.join("a.txt"), b"alpha").unwrap();
        fs::create_dir_all(root.join("sub")).unwrap();
        fs::write(root.join("sub/b.txt"), "你好，世界".as_bytes()).unwrap();

        let mut cs = CheckpointStore::open(root, store).unwrap();
        let snap = read_workspace(root, store);
        let id = cs.create("initial").unwrap();

        // mutate: change a, add c, delete sub/b
        fs::write(root.join("a.txt"), b"ALPHA CHANGED").unwrap();
        fs::write(root.join("c.txt"), b"new file").unwrap();
        fs::remove_file(root.join("sub/b.txt")).unwrap();

        // restore -> must match the original snapshot exactly
        cs.restore(&id).unwrap();
        let after = read_workspace(root, store);
        assert_eq!(snap, after, "workspace not byte-identical after restore");
        // the extra file must be gone
        assert!(!root.join("c.txt").exists());
        // the deleted file must be back with original content
        assert_eq!(
            fs::read(root.join("sub/b.txt")).unwrap(),
            "你好，世界".as_bytes()
        );
    }

    #[test]
    fn list_reports_metadata() {
        let ws = TempDir::new("ws");
        let st = TempDir::new("store");
        fs::write(ws.path().join("x"), b"1").unwrap();
        let mut cs = CheckpointStore::open(ws.path(), st.path()).unwrap();
        cs.create("first").unwrap();
        fs::write(ws.path().join("y"), b"2").unwrap();
        cs.create("second").unwrap();
        let list = cs.list();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].label, "first");
        assert_eq!(list[0].file_count, 1);
        assert_eq!(list[1].label, "second");
        assert_eq!(list[1].file_count, 2);
    }

    #[test]
    fn undo_and_redo_move_between_states() {
        let ws = TempDir::new("ws");
        let st = TempDir::new("store");
        let root = ws.path();

        let mut cs = CheckpointStore::open(root, st.path()).unwrap();

        fs::write(root.join("f.txt"), b"v1").unwrap();
        let c1 = cs.create("v1").unwrap();
        fs::write(root.join("f.txt"), b"v2").unwrap();
        let c2 = cs.create("v2").unwrap();

        // currently matches c2
        assert_eq!(cs.current(), Some(&c2));

        // undo -> back to v1 state
        let u = cs.undo().unwrap();
        assert_eq!(u, Some(c1.clone()));
        assert_eq!(fs::read(root.join("f.txt")).unwrap(), b"v1");
        assert_eq!(cs.current(), Some(&c1));

        // nothing more to undo (only one state left)
        assert_eq!(cs.undo().unwrap(), None);

        // redo -> forward to v2
        let r = cs.redo().unwrap();
        assert_eq!(r, Some(c2.clone()));
        assert_eq!(fs::read(root.join("f.txt")).unwrap(), b"v2");

        // nothing more to redo
        assert_eq!(cs.redo().unwrap(), None);
    }

    #[test]
    fn create_clears_redo_stack() {
        let ws = TempDir::new("ws");
        let st = TempDir::new("store");
        let root = ws.path();
        let mut cs = CheckpointStore::open(root, st.path()).unwrap();

        fs::write(root.join("f"), b"a").unwrap();
        cs.create("a").unwrap();
        fs::write(root.join("f"), b"b").unwrap();
        cs.create("b").unwrap();
        cs.undo().unwrap(); // redo stack now has the "b" state

        fs::write(root.join("f"), b"c").unwrap();
        cs.create("c").unwrap(); // should clear redo

        assert_eq!(cs.redo().unwrap(), None, "redo must be cleared by create");
    }

    #[test]
    fn manifests_reload_after_reopen() {
        let ws = TempDir::new("ws");
        let st = TempDir::new("store");
        let root = ws.path();
        {
            let mut cs = CheckpointStore::open(root, st.path()).unwrap();
            fs::write(root.join("f"), b"data").unwrap();
            cs.create("persisted").unwrap();
        }
        // reopen: the manifest should still be listed and restorable
        let cs2 = CheckpointStore::open(root, st.path()).unwrap();
        let list = cs2.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].label, "persisted");
    }

    #[test]
    fn restore_unknown_id_is_not_found() {
        let ws = TempDir::new("ws");
        let st = TempDir::new("store");
        let mut cs = CheckpointStore::open(ws.path(), st.path()).unwrap();
        let err = cs
            .restore(&CheckpointId::from_existing("ckpt_does_not_exist"))
            .unwrap_err();
        assert!(err.is(na_common::ErrorKind::NotFound));
    }

    #[test]
    fn manifest_rejects_object_path_traversal_and_reserved_targets() {
        let ws = TempDir::new("ws-bad-manifest");
        let st = TempDir::new("store-bad-manifest");
        fs::create_dir_all(st.path().join("objects")).unwrap();
        let manifest = CheckpointManifest {
            id: CheckpointId::new(),
            label: "crafted".into(),
            created_ms: now_millis(),
            files: vec![("leak.md".into(), "../../../outside-file".into())],
        };
        fs::write(
            st.path().join("checkpoints.jsonl"),
            format!("{}\n", serde_json::to_string(&manifest).unwrap()),
        )
        .unwrap();

        let error = match CheckpointStore::open(ws.path(), st.path()) {
            Ok(_) => panic!("crafted manifest should be rejected"),
            Err(error) => error,
        };
        assert!(error.is(na_common::ErrorKind::Serialization), "{error}");
    }

    #[test]
    fn corrupt_blob_aborts_restore_before_workspace_mutation() {
        let ws = TempDir::new("ws-corrupt-blob");
        let st = TempDir::new("store-corrupt-blob");
        let file = ws.path().join("chapter.md");
        fs::write(&file, "snapshot").unwrap();
        let mut store = CheckpointStore::open(ws.path(), st.path()).unwrap();
        let checkpoint = store.create("snapshot").unwrap();
        let hash = store.manifests[0].files[0].1.clone();
        fs::write(st.path().join("objects").join(hash), "corrupt").unwrap();
        fs::write(&file, "current unsaved work").unwrap();

        let error = store.restore(&checkpoint).unwrap_err();
        assert!(error.is(na_common::ErrorKind::Serialization), "{error}");
        assert_eq!(fs::read_to_string(file).unwrap(), "current unsaved work");
    }

    #[test]
    fn failed_delete_persistence_restores_in_memory_manifest() {
        let ws = TempDir::new("ws-delete-failure");
        let st = TempDir::new("store-delete-failure");
        fs::write(ws.path().join("chapter.md"), "snapshot").unwrap();
        let mut store = CheckpointStore::open(ws.path(), st.path()).unwrap();
        let checkpoint = store.create("keep").unwrap();
        fs::remove_file(&store.manifest_path).unwrap();
        fs::create_dir(&store.manifest_path).unwrap();

        assert!(store.delete(&checkpoint).is_err());
        assert_eq!(store.list().len(), 1);
        assert_eq!(store.list()[0].id, checkpoint);
    }

    #[test]
    fn git_dir_is_ignored() {
        let ws = TempDir::new("ws");
        let st = TempDir::new("store");
        let root = ws.path();
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::write(root.join(".git/HEAD"), b"ref: refs/heads/main").unwrap();
        fs::write(root.join("real.txt"), b"keep me").unwrap();

        let mut cs = CheckpointStore::open(root, st.path()).unwrap();
        let id = cs.create("snap").unwrap();
        let manifest = cs.manifests.iter().find(|m| m.id == id).unwrap();
        assert_eq!(manifest.files.len(), 1, ".git contents must be skipped");
        assert_eq!(manifest.files[0].0, "real.txt");
    }

    #[test]
    fn all_reserved_state_directories_are_skipped_and_store_reopens() {
        let ws = TempDir::new("ws-reserved");
        let st = TempDir::new("store-reserved");
        for directory in [".na-vcs", ".NA", ".GIT"] {
            fs::create_dir_all(ws.path().join(directory)).unwrap();
            fs::write(ws.path().join(directory).join("state"), "internal").unwrap();
        }
        fs::write(ws.path().join("chapter.md"), "manuscript").unwrap();
        {
            let mut store = CheckpointStore::open(ws.path(), st.path()).unwrap();
            let checkpoint = store.create("clean").unwrap();
            let manifest = store
                .manifests
                .iter()
                .find(|manifest| manifest.id == checkpoint)
                .unwrap();
            assert_eq!(manifest.files.len(), 1);
            assert_eq!(manifest.files[0].0, "chapter.md");
        }

        assert_eq!(
            CheckpointStore::open(ws.path(), st.path())
                .unwrap()
                .list()
                .len(),
            1
        );
    }

    #[cfg(unix)]
    fn symlink_dir(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }

    #[cfg(windows)]
    fn symlink_dir(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::windows::fs::symlink_dir(target, link)
    }

    #[cfg(unix)]
    fn symlink_file(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }

    #[cfg(windows)]
    fn symlink_file(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::windows::fs::symlink_file(target, link)
    }

    #[test]
    fn restore_rejects_parent_symlink_escape() {
        let ws = TempDir::new("ws-link");
        let st = TempDir::new("store-link");
        let outside = TempDir::new("outside-link");
        let root = ws.path();
        fs::create_dir_all(root.join("chapter")).unwrap();
        fs::write(root.join("chapter/one.md"), "snapshot").unwrap();
        let mut store = CheckpointStore::open(root, st.path()).unwrap();
        let checkpoint = store.create("before-link").unwrap();

        fs::remove_dir_all(root.join("chapter")).unwrap();
        let link = root.join("chapter");
        if let Err(error) = symlink_dir(outside.path(), &link) {
            if error.kind() == std::io::ErrorKind::PermissionDenied {
                return;
            }
            panic!("creating test symlink failed: {error}");
        }

        let error = store.restore(&checkpoint).unwrap_err();
        assert!(error.is(na_common::ErrorKind::SandboxViolation), "{error}");
        assert!(!outside.path().join("one.md").exists());
        let _ = fs::remove_file(link);
    }

    #[test]
    fn failed_undo_keeps_history_on_the_current_checkpoint() {
        let ws = TempDir::new("ws-undo-link");
        let st = TempDir::new("store-undo-link");
        let outside = TempDir::new("outside-undo-link");
        let root = ws.path();
        fs::create_dir_all(root.join("chapter")).unwrap();
        fs::write(root.join("chapter/one.md"), "v1").unwrap();
        let mut store = CheckpointStore::open(root, st.path()).unwrap();
        store.create("v1").unwrap();
        fs::write(root.join("chapter/one.md"), "v2").unwrap();
        let current = store.create("v2").unwrap();

        fs::remove_dir_all(root.join("chapter")).unwrap();
        let link = root.join("chapter");
        if let Err(error) = symlink_dir(outside.path(), &link) {
            if error.kind() == std::io::ErrorKind::PermissionDenied {
                return;
            }
            panic!("creating test symlink failed: {error}");
        }

        assert!(store.undo().is_err());
        assert_eq!(store.current(), Some(&current));
        assert!(store.redo_stack.is_empty());
        assert!(!outside.path().join("one.md").exists());
        let _ = fs::remove_file(link);
    }

    #[test]
    fn restore_never_follows_leaf_symlink() {
        let ws = TempDir::new("ws-leaf-link");
        let st = TempDir::new("store-leaf-link");
        let outside = TempDir::new("outside-leaf-link");
        let root = ws.path();
        fs::write(root.join("chapter.md"), "snapshot").unwrap();
        let mut store = CheckpointStore::open(root, st.path()).unwrap();
        let checkpoint = store.create("snapshot").unwrap();

        fs::remove_file(root.join("chapter.md")).unwrap();
        let target = outside.path().join("target.md");
        fs::write(&target, "outside").unwrap();
        let link = root.join("chapter.md");
        if let Err(error) = symlink_file(&target, &link) {
            if error.kind() == std::io::ErrorKind::PermissionDenied {
                return;
            }
            panic!("creating test symlink failed: {error}");
        }

        let error = store.restore(&checkpoint).unwrap_err();
        assert!(error.is(na_common::ErrorKind::SandboxViolation), "{error}");
        assert_eq!(fs::read_to_string(target).unwrap(), "outside");
        assert!(fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        let _ = fs::remove_file(link);
    }

    #[test]
    fn restore_removes_extra_symlink_without_touching_target() {
        let ws = TempDir::new("ws-extra-link");
        let st = TempDir::new("store-extra-link");
        let outside = TempDir::new("outside-extra-link");
        let root = ws.path();
        fs::write(root.join("kept.md"), "snapshot").unwrap();
        let mut store = CheckpointStore::open(root, st.path()).unwrap();
        let checkpoint = store.create("snapshot").unwrap();

        let target = outside.path().join("target.md");
        fs::write(&target, "outside").unwrap();
        let link = root.join("extra.md");
        if let Err(error) = symlink_file(&target, &link) {
            if error.kind() == std::io::ErrorKind::PermissionDenied {
                return;
            }
            panic!("creating test symlink failed: {error}");
        }

        store.restore(&checkpoint).unwrap();
        assert!(fs::symlink_metadata(&link).is_err());
        assert_eq!(fs::read_to_string(target).unwrap(), "outside");
    }

    #[test]
    fn restore_handles_file_directory_type_changes() {
        let ws = TempDir::new("ws-type-changes");
        let st = TempDir::new("store-type-changes");
        let root = ws.path();
        fs::write(root.join("as_file"), "file snapshot").unwrap();
        fs::create_dir(root.join("as_dir")).unwrap();
        fs::write(root.join("as_dir/chapter.md"), "nested snapshot").unwrap();
        let mut store = CheckpointStore::open(root, st.path()).unwrap();
        let checkpoint = store.create("snapshot").unwrap();

        fs::remove_file(root.join("as_file")).unwrap();
        fs::create_dir(root.join("as_file")).unwrap();
        fs::write(root.join("as_file/extra.md"), "extra").unwrap();
        fs::remove_dir_all(root.join("as_dir")).unwrap();
        fs::write(root.join("as_dir"), "blocking file").unwrap();

        store.restore(&checkpoint).unwrap();
        assert_eq!(
            fs::read_to_string(root.join("as_file")).unwrap(),
            "file snapshot"
        );
        assert!(root.join("as_dir").is_dir());
        assert_eq!(
            fs::read_to_string(root.join("as_dir/chapter.md")).unwrap(),
            "nested snapshot"
        );
    }

    #[test]
    fn protected_state_blocks_directory_replacement_before_mutation() {
        let ws = TempDir::new("ws-protected-blocker");
        let st = TempDir::new("store-protected-blocker");
        let root = ws.path();
        fs::write(root.join("container"), "snapshot").unwrap();
        fs::write(root.join("other.md"), "snapshot").unwrap();
        let mut store = CheckpointStore::open(root, st.path()).unwrap();
        let checkpoint = store.create("snapshot").unwrap();

        fs::remove_file(root.join("container")).unwrap();
        fs::create_dir_all(root.join("container/.na-vcs")).unwrap();
        fs::write(root.join("container/.na-vcs/state"), "protected").unwrap();
        fs::write(root.join("other.md"), "current").unwrap();
        fs::write(root.join("extra.md"), "must survive failure").unwrap();

        let error = store.restore(&checkpoint).unwrap_err();
        assert!(error.is(na_common::ErrorKind::Conflict), "{error}");
        assert_eq!(
            fs::read_to_string(root.join("other.md")).unwrap(),
            "current"
        );
        assert_eq!(
            fs::read_to_string(root.join("extra.md")).unwrap(),
            "must survive failure"
        );
        assert_eq!(
            fs::read_to_string(root.join("container/.na-vcs/state")).unwrap(),
            "protected"
        );
    }

    #[test]
    fn manifest_rejects_ancestor_conflicts() {
        let ws = TempDir::new("ws-ancestor-conflict");
        let st = TempDir::new("store-ancestor-conflict");
        fs::create_dir_all(st.path().join("objects")).unwrap();
        let hash = content_hash(b"data");
        let manifest = CheckpointManifest {
            id: CheckpointId::new(),
            label: "crafted".into(),
            created_ms: now_millis(),
            files: vec![
                ("chapter".into(), hash.clone()),
                ("chapter/one.md".into(), hash),
            ],
        };
        fs::write(
            st.path().join("checkpoints.jsonl"),
            format!("{}\n", serde_json::to_string(&manifest).unwrap()),
        )
        .unwrap();

        let error = match CheckpointStore::open(ws.path(), st.path()) {
            Ok(_) => panic!("ancestor conflict should be rejected"),
            Err(error) => error,
        };
        assert!(error.is(na_common::ErrorKind::Serialization), "{error}");
    }

    #[test]
    fn legacy_reserved_entries_load_but_restore_preserves_current_state() {
        let ws = TempDir::new("ws-legacy-reserved");
        let st = TempDir::new("store-legacy-reserved");
        fs::create_dir_all(st.path().join("objects")).unwrap();
        fs::create_dir_all(ws.path().join("nested/.na-vcs")).unwrap();
        fs::write(ws.path().join("nested/.na-vcs/state"), "current").unwrap();
        let legacy_hash = content_hash(b"legacy");
        write_content_object(&st.path().join("objects"), &legacy_hash, b"legacy").unwrap();
        let manifest = CheckpointManifest {
            id: CheckpointId::new(),
            label: "legacy".into(),
            created_ms: now_millis(),
            files: vec![("nested/.na-vcs/state".into(), legacy_hash)],
        };
        fs::write(
            st.path().join("checkpoints.jsonl"),
            format!("{}\n", serde_json::to_string(&manifest).unwrap()),
        )
        .unwrap();

        let mut store = CheckpointStore::open(ws.path(), st.path()).unwrap();
        store.restore(&manifest.id).unwrap();
        assert_eq!(
            fs::read_to_string(ws.path().join("nested/.na-vcs/state")).unwrap(),
            "current"
        );
    }

    #[test]
    fn manifest_symlinks_are_rejected_including_dangling_links() {
        for dangling in [false, true] {
            let ws = TempDir::new("ws-manifest-link");
            let st = TempDir::new("store-manifest-link");
            fs::create_dir_all(st.path().join("objects")).unwrap();
            let target = st.path().join("outside.jsonl");
            if !dangling {
                fs::write(&target, "outside").unwrap();
            }
            let link = st.path().join("checkpoints.jsonl");
            if let Err(error) = symlink_file(&target, &link) {
                if error.kind() == std::io::ErrorKind::PermissionDenied {
                    return;
                }
                panic!("creating test symlink failed: {error}");
            }

            let error = match CheckpointStore::open(ws.path(), st.path()) {
                Ok(_) => panic!("manifest symlink should be rejected"),
                Err(error) => error,
            };
            assert!(error.is(na_common::ErrorKind::SandboxViolation), "{error}");
            if !dangling {
                assert_eq!(fs::read_to_string(target).unwrap(), "outside");
            }
            let _ = fs::remove_file(link);
        }
    }

    #[cfg(unix)]
    #[test]
    fn create_rejects_non_utf8_unrepresentable_file_name() {
        use std::os::unix::ffi::OsStringExt;

        let ws = TempDir::new("ws-non-utf8");
        let st = TempDir::new("store-non-utf8");
        let name = std::ffi::OsString::from_vec(vec![b'f', 0xff]);
        fs::write(ws.path().join(name), "data").unwrap();
        let mut store = CheckpointStore::open(ws.path(), st.path()).unwrap();

        let error = store.create("snapshot").unwrap_err();
        assert!(error.is(na_common::ErrorKind::InvalidInput), "{error}");
        assert!(store.list().is_empty());
    }
}
