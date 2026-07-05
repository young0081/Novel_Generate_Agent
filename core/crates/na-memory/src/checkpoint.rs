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

use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Component, Path, PathBuf};

use na_common::time::now_millis;
use na_common::{CheckpointId, CoreError, Result};
use serde::{Deserialize, Serialize};

/// Compute a stable content key for `bytes`.
///
/// We combine the byte length with a 64-bit FNV-1a hash and render both as hex
/// (`"{len:x}-{fnv:x}"`). Pairing the length with the hash makes accidental
/// collisions astronomically unlikely for the file sizes we deal with, while
/// staying dependency-free (no `sha2`).
pub fn content_hash(bytes: &[u8]) -> String {
    // FNV-1a 64-bit constants.
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(PRIME);
    }
    format!("{:x}-{:x}", bytes.len(), hash)
}

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
        let workspace_root = workspace_root.as_ref().to_path_buf();
        let store_dir = store_dir.as_ref().to_path_buf();
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

        let id = CheckpointId::new();
        let manifest = CheckpointManifest {
            id: id.clone(),
            label: label.to_string(),
            created_ms: now_millis(),
            files,
        };
        self.append_manifest(&manifest)?;
        self.manifests.push(manifest);

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
        let current = self.undo_stack.pop().expect("len checked >= 2");
        self.redo_stack.push(current);
        let target = self
            .undo_stack
            .last()
            .cloned()
            .expect("at least one remains");
        self.restore_without_history(&target)?;
        Ok(Some(target))
    }

    /// Re-apply a state previously undone. Returns the id now matched, or `None`
    /// if there is nothing to redo.
    pub fn redo(&mut self) -> Result<Option<CheckpointId>> {
        let Some(target) = self.redo_stack.pop() else {
            return Ok(None);
        };
        self.undo_stack.push(target.clone());
        self.restore_without_history(&target)?;
        Ok(Some(target))
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
        let before = self.manifests.len();
        self.manifests.retain(|m| &m.id != id);
        if self.manifests.len() == before {
            return Err(CoreError::not_found(format!("checkpoint {id} not found")));
        }
        self.persist_manifests()?;
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
        use std::collections::BTreeSet;

        // Desired relative paths (normalized to forward slashes in the manifest).
        let desired: BTreeSet<&str> = manifest.files.iter().map(|(p, _)| p.as_str()).collect();

        // 1. Delete workspace files not present in the snapshot.
        let mut existing: Vec<String> = Vec::new();
        self.collect_rel_files(&self.workspace_root, &mut existing)?;
        for rel in &existing {
            if !desired.contains(rel.as_str()) {
                let abs = self.workspace_root.join(rel_to_pathbuf(rel));
                if abs.exists() {
                    fs::remove_file(&abs).map_err(|e| {
                        CoreError::from(e).with_context(format!("deleting {rel} during restore"))
                    })?;
                }
            }
        }

        // 2. Write / overwrite files from their blobs.
        for (rel, hash) in &manifest.files {
            let blob = self.read_blob(hash)?;
            let abs = self.workspace_root.join(rel_to_pathbuf(rel));
            if let Some(parent) = abs.parent() {
                fs::create_dir_all(parent).map_err(|e| {
                    CoreError::from(e).with_context(format!("creating dirs for {rel}"))
                })?;
            }
            fs::write(&abs, &blob).map_err(|e| {
                CoreError::from(e).with_context(format!("writing {rel} during restore"))
            })?;
        }

        // 3. Prune now-empty directories (best effort, deepest first).
        self.prune_empty_dirs(&self.workspace_root)?;
        Ok(())
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
                let bytes = fs::read(&path).map_err(|e| {
                    CoreError::from(e).with_context(format!("reading file {}", path.display()))
                })?;
                let hash = content_hash(&bytes);
                self.write_blob_if_absent(&hash, &bytes)?;
                let rel = self.rel_path(&path)?;
                out.push((rel, hash));
            }
            // symlinks and other special files are skipped intentionally.
        }
        Ok(())
    }

    /// Collect the relative paths of all (non-ignored) files under `dir`.
    fn collect_rel_files(&self, dir: &Path, out: &mut Vec<String>) -> Result<()> {
        if !dir.exists() {
            return Ok(());
        }
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
                self.collect_rel_files(&path, out)?;
            } else if file_type.is_file() {
                out.push(self.rel_path(&path)?);
            }
        }
        Ok(())
    }

    /// True if `path` is the store dir, inside it, or a `.git` directory.
    fn is_ignored(&self, path: &Path) -> bool {
        if paths_equal(path, &self.store_dir) || path.starts_with(&self.store_dir) {
            return true;
        }
        matches!(path.file_name().and_then(|n| n.to_str()), Some(".git"))
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
                Component::Normal(os) => parts.push(os.to_string_lossy().into_owned()),
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

    fn blob_path(&self, hash: &str) -> PathBuf {
        self.objects_dir.join(hash)
    }

    fn write_blob_if_absent(&self, hash: &str, bytes: &[u8]) -> Result<()> {
        let path = self.blob_path(hash);
        if path.exists() {
            return Ok(());
        }
        fs::write(&path, bytes)
            .map_err(|e| CoreError::from(e).with_context(format!("writing blob {hash}")))?;
        Ok(())
    }

    fn read_blob(&self, hash: &str) -> Result<Vec<u8>> {
        let path = self.blob_path(hash);
        fs::read(&path).map_err(|e| {
            CoreError::from(e).with_context(format!("reading blob {hash} (corrupt store?)"))
        })
    }

    fn append_manifest(&self, manifest: &CheckpointManifest) -> Result<()> {
        let line = serde_json::to_string(manifest)?;
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.manifest_path)
            .map_err(|e| CoreError::from(e).with_context("opening checkpoints.jsonl for append"))?;
        file.write_all(line.as_bytes())
            .and_then(|_| file.write_all(b"\n"))
            .map_err(|e| CoreError::from(e).with_context("appending checkpoint manifest"))?;
        Ok(())
    }

    /// Rewrite the whole manifest log from the current set (used after a delete).
    /// Writes a temp file then renames for atomicity.
    fn persist_manifests(&self) -> Result<()> {
        let tmp = self.manifest_path.with_extension("jsonl.tmp");
        {
            let mut file = fs::File::create(&tmp)
                .map_err(|e| CoreError::from(e).with_context("creating checkpoints temp file"))?;
            for m in &self.manifests {
                let line = serde_json::to_string(m)?;
                file.write_all(line.as_bytes())
                    .and_then(|_| file.write_all(b"\n"))
                    .map_err(|e| {
                        CoreError::from(e).with_context("writing checkpoints temp file")
                    })?;
            }
            file.flush()
                .map_err(|e| CoreError::from(e).with_context("flushing checkpoints temp file"))?;
        }
        fs::rename(&tmp, &self.manifest_path)
            .map_err(|e| CoreError::from(e).with_context("replacing checkpoints.jsonl"))?;
        Ok(())
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

    /// Recursively remove empty directories under `dir` (but never `dir` itself
    /// or the store dir). Best-effort: errors are ignored so a restore is not
    /// derailed by a directory we cannot remove.
    fn prune_empty_dirs(&self, dir: &Path) -> Result<()> {
        let read = match fs::read_dir(dir) {
            Ok(r) => r,
            Err(_) => return Ok(()),
        };
        let mut subdirs: Vec<PathBuf> = Vec::new();
        for entry in read.flatten() {
            let path = entry.path();
            if self.is_ignored(&path) {
                continue;
            }
            if path.is_dir() {
                subdirs.push(path);
            }
        }
        for sub in subdirs {
            self.prune_empty_dirs(&sub)?;
            // Try to remove if it is now empty.
            if let Ok(mut it) = fs::read_dir(&sub) {
                if it.next().is_none() {
                    let _ = fs::remove_dir(&sub);
                }
            }
        }
        Ok(())
    }
}

/// Convert a forward-slash relative path into a native [`PathBuf`].
fn rel_to_pathbuf(rel: &str) -> PathBuf {
    let mut pb = PathBuf::new();
    for part in rel.split('/') {
        if !part.is_empty() {
            pb.push(part);
        }
    }
    pb
}

/// Compare two paths by their canonical form when possible, falling back to a
/// literal comparison (canonicalize fails if the path does not exist yet).
fn paths_equal(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(ca), Ok(cb)) => ca == cb,
        _ => a == b,
    }
}

/// Read and parse all manifests from `path` (missing file => empty vec). Stored
/// in a `BTreeMap` keyed by id first so a re-appended id (should not happen, but
/// be defensive) keeps the last version, then flattened preserving file order by
/// `created_ms`.
fn load_manifests(path: &Path) -> Result<Vec<CheckpointManifest>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file = fs::File::open(path)
        .map_err(|e| CoreError::from(e).with_context("opening checkpoints.jsonl"))?;
    let reader = BufReader::new(file);
    let mut by_id: BTreeMap<String, CheckpointManifest> = BTreeMap::new();
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
        by_id.insert(manifest.id.0.clone(), manifest);
    }
    let mut out: Vec<CheckpointManifest> = by_id.into_values().collect();
    out.sort_by_key(|m| m.created_ms);
    Ok(out)
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
}
