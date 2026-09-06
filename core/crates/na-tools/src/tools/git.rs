//! A literature-friendly version store ("fiction VCS").
//!
//! Instead of shelling out to real `git`, this is a small pure-Rust snapshot
//! store under a `.na-vcs/` directory in the workspace. It is tuned for prose:
//! every commit records each file's content *and word count* (CJK-aware), and
//! [`FictionVcs::log`] surfaces the per-commit total and the word *delta* from
//! the previous commit so an author can see "+1,240 words" at a glance.
//!
//! Public operations:
//! * [`commit`](FictionVcs::commit) — snapshot all workspace files, return an id.
//! * [`log`](FictionVcs::log) — history (of the current branch) with timestamps,
//!   totals, and deltas.
//! * [`diff`](FictionVcs::diff) — per-file added/removed/changed summary with
//!   word deltas, versus the previous commit on the same branch.
//! * [`diff_lines`](FictionVcs::diff_lines) — a structured **line-level** diff of
//!   one file between any two revisions (LCS-based added/removed/context).
//! * [`restore`](FictionVcs::restore) — restore one file or the whole workspace
//!   to a commit.
//! * [`chapter_history`](FictionVcs::chapter_history) — the word-count trail of a
//!   single file across all commits.
//! * [`branch`](FictionVcs::branch) / [`switch`](FictionVcs::switch) /
//!   [`branches`](FictionVcs::branches) — fork and explore **alternate plot
//!   lines**; commits are recorded per branch.
//! * [`tag`](FictionVcs::tag) / [`tags`](FictionVcs::tags) — label milestone
//!   commits (e.g. `"卷一终"`).
//!
//! The [`Tool`]s ([`GitCommitTool`], [`GitLogTool`], [`GitDiffTool`],
//! [`GitRestoreTool`], [`GitBranchTool`]) expose this over the standard tool
//! protocol; the mutating ones require [`Capability::GitWrite`].

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

use na_common::time::now_millis;
use na_common::{json, CheckpointId, CoreError, Json, Result};
use na_memory::{content_hash, read_content_object, validate_content_hash, write_content_object};
use na_sandbox::{Capability, PathJail};
use serde::{Deserialize, Serialize};

use super::fs::atomic_write;
use crate::tool::{BoxFuture, Tool, ToolContext, ToolResult, ToolSpec};

/// The directory (under the workspace root) holding the fiction VCS state.
const VCS_DIR: &str = ".na-vcs";

/// Count words in `text` with CJK awareness.
///
/// Each CJK ideograph counts as one word (Chinese prose has no spaces), and each
/// whitespace-delimited run of non-CJK characters counts as one word. So
/// `"Hello 世界"` is 1 (English word) + 2 (CJK chars) = 3.
pub fn count_words(text: &str) -> usize {
    let mut words = 0usize;
    let mut in_ascii_word = false;
    for ch in text.chars() {
        if is_cjk(ch) {
            // CJK char: counts on its own, and ends any ASCII word run.
            words += 1;
            in_ascii_word = false;
        } else if ch.is_whitespace() {
            in_ascii_word = false;
        } else {
            // part of a non-CJK word
            if !in_ascii_word {
                words += 1;
                in_ascii_word = true;
            }
        }
    }
    words
}

/// CJK ideograph test, matching na-memory's tokenizer coverage.
fn is_cjk(c: char) -> bool {
    let u = c as u32;
    (0x4E00..=0x9FFF).contains(&u)
        || (0x3400..=0x4DBF).contains(&u)
        || (0xF900..=0xFAFF).contains(&u)
        || (0x20000..=0x2A6DF).contains(&u)
}

/// A file as recorded in a commit: its content hash and word count.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileRecord {
    /// Workspace-relative path (forward slashes).
    pub path: String,
    /// Content hash (na-memory's length-tagged FNV-1a).
    pub hash: String,
    /// CJK-aware word count of the content.
    pub words: usize,
}

/// One committed snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Commit {
    /// Commit id (reuses the [`CheckpointId`] newtype for a typed handle).
    pub id: CheckpointId,
    /// Author-supplied message.
    pub message: String,
    /// Creation time (epoch ms).
    pub ts: u64,
    /// The branch this commit was recorded on. Defaults to `"main"` for commits
    /// written before branching existed (kept backward-compatible via serde).
    #[serde(default = "default_branch")]
    pub branch: String,
    /// Files in the snapshot, sorted by path.
    pub files: Vec<FileRecord>,
}

/// The name of the default branch.
pub(crate) fn default_branch() -> String {
    "main".to_string()
}

impl Commit {
    /// Total words across all files in this commit.
    pub fn total_words(&self) -> usize {
        self.files.iter().map(|f| f.words).sum()
    }
}

/// A single row of [`FictionVcs::log`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogEntry {
    pub id: CheckpointId,
    pub message: String,
    pub ts: u64,
    pub total_words: usize,
    /// Word change versus the previous commit (can be negative).
    pub word_delta: i64,
    pub file_count: usize,
}

/// Per-file diff line versus the previous commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileDiff {
    pub path: String,
    /// "added", "removed", "changed", or "unchanged".
    pub status: String,
    /// Word count in this commit (0 if removed).
    pub words: usize,
    /// Word delta versus the previous commit for this file.
    pub word_delta: i64,
}

/// The result of [`FictionVcs::diff`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffSummary {
    pub id: CheckpointId,
    pub message: String,
    /// Previous commit id, if any.
    pub parent: Option<CheckpointId>,
    pub files: Vec<FileDiff>,
    /// Net word change across the whole commit.
    pub total_word_delta: i64,
}

/// One point in a single chapter's word-count history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChapterPoint {
    pub commit: CheckpointId,
    pub ts: u64,
    /// Word count of the chapter at that commit (0 if absent).
    pub words: usize,
    /// Whether the file existed in that commit.
    pub present: bool,
}

/// A single hunk of a line-level diff.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffLine {
    /// `"added"`, `"removed"`, or `"context"` (unchanged, shown for orientation).
    pub kind: String,
    /// 1-based line number in revision A (`None` for added lines).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub a_line: Option<usize>,
    /// 1-based line number in revision B (`None` for removed lines).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub b_line: Option<usize>,
    /// The text of the line (without its trailing newline).
    pub text: String,
}

/// A structured line-level diff of one file between two revisions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineDiff {
    /// Workspace-relative path that was diffed.
    pub path: String,
    /// Source revision id.
    pub rev_a: CheckpointId,
    /// Target revision id.
    pub rev_b: CheckpointId,
    /// The ordered diff lines (added / removed / context).
    pub lines: Vec<DiffLine>,
    /// Count of added lines.
    pub added: usize,
    /// Count of removed lines.
    pub removed: usize,
}

/// A named tag pointing at a commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tag {
    /// The label (e.g. `"v1-draft"`, `"卷一终"`).
    pub label: String,
    /// The commit the tag points at.
    pub commit: CheckpointId,
}

/// Persisted branch/tag bookkeeping stored alongside the commit log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct VcsState {
    /// The currently checked-out branch.
    current_branch: String,
    /// All known branch names (includes `current_branch` and `main`).
    branches: Vec<String>,
    /// Tags by insertion order.
    tags: Vec<Tag>,
}

impl Default for VcsState {
    fn default() -> Self {
        VcsState {
            current_branch: default_branch(),
            branches: vec![default_branch()],
            tags: Vec::new(),
        }
    }
}

/// The pure-Rust fiction version store rooted at a workspace.
#[derive(Debug)]
pub struct FictionVcs {
    jail: PathJail,
    workspace_root: PathBuf,
    vcs_dir: PathBuf,
    objects_dir: PathBuf,
    commits_path: PathBuf,
    state_path: PathBuf,
    commits: Vec<Commit>,
    state: VcsState,
}

#[derive(Debug)]
struct WorkspaceEntry {
    rel: String,
    is_dir: bool,
}

impl FictionVcs {
    /// Open (or initialize) the store under `<workspace_root>/.na-vcs`.
    pub fn open(workspace_root: impl AsRef<Path>) -> Result<Self> {
        let jail = PathJail::new(workspace_root)?;
        let workspace_root = jail.root().to_path_buf();
        let vcs_dir = workspace_root.join(VCS_DIR);
        let objects_dir = vcs_dir.join("objects");
        let commits_path = vcs_dir.join("commits.jsonl");
        let state_path = vcs_dir.join("state.json");

        ensure_store_directory(&vcs_dir, ".na-vcs")?;
        ensure_store_directory(&objects_dir, ".na-vcs/objects")?;
        ensure_metadata_file_or_missing(&commits_path, ".na-vcs/commits.jsonl")?;
        ensure_metadata_file_or_missing(&state_path, ".na-vcs/state.json")?;

        let commits = load_commits(&commits_path)?;
        let mut state = load_state(&state_path)?;
        validate_state(&state, &commits)?;
        // Self-heal: ensure every branch referenced by a commit is listed, so a
        // store created before branching gains a coherent `main` branch and any
        // hand-edited log stays consistent.
        for c in &commits {
            if !state.branches.contains(&c.branch) {
                state.branches.push(c.branch.clone());
            }
        }
        if !state.branches.contains(&state.current_branch) {
            state.branches.push(state.current_branch.clone());
        }

        Ok(FictionVcs {
            jail,
            workspace_root,
            vcs_dir,
            objects_dir,
            commits_path,
            state_path,
            commits,
            state,
        })
    }

    /// Snapshot every workspace file (skipping `.na-vcs`, `.na`, and `.git`) and
    /// record a new commit. Returns its id.
    pub fn commit(&mut self, message: &str) -> Result<CheckpointId> {
        let mut files: Vec<FileRecord> = Vec::new();
        let root = self.workspace_root.clone();
        self.snapshot_dir(&root, &mut files)?;
        files.sort_by(|a, b| a.path.cmp(&b.path));
        validate_file_records(&files)?;

        let id = CheckpointId::new();
        let commit = Commit {
            id: id.clone(),
            message: message.to_string(),
            ts: now_millis(),
            branch: self.state.current_branch.clone(),
            files,
        };
        self.append_commit(&commit)?;
        self.commits.push(commit);
        Ok(id)
    }

    /// History of the **current branch**, oldest-first, with totals and word
    /// deltas. (Before any branching is used, every commit is on `main`, so this
    /// is the full history.)
    pub fn log(&self) -> Vec<LogEntry> {
        self.log_for(&self.state.current_branch)
    }

    /// History of a specific `branch`, oldest-first, with totals and word deltas.
    pub fn log_for(&self, branch: &str) -> Vec<LogEntry> {
        let mut out = Vec::new();
        let mut prev_total = 0i64;
        for c in self.commits.iter().filter(|c| c.branch == branch) {
            let total = c.total_words() as i64;
            out.push(LogEntry {
                id: c.id.clone(),
                message: c.message.clone(),
                ts: c.ts,
                total_words: c.total_words(),
                word_delta: total - prev_total,
                file_count: c.files.len(),
            });
            prev_total = total;
        }
        out
    }

    /// Per-file diff of commit `id` against its immediate predecessor **on the
    /// same branch**. (On a linear, single-branch history this is just the
    /// previous commit, matching the original behavior.)
    pub fn diff(&self, id: &CheckpointId) -> Result<DiffSummary> {
        let idx = self.index_of(id)?;
        let commit = &self.commits[idx];
        // The parent is the latest earlier commit sharing this commit's branch.
        let parent = self.commits[..idx]
            .iter()
            .rev()
            .find(|c| c.branch == commit.branch);

        let cur: BTreeMap<&str, &FileRecord> =
            commit.files.iter().map(|f| (f.path.as_str(), f)).collect();
        let prev: BTreeMap<&str, &FileRecord> = parent
            .map(|p| p.files.iter().map(|f| (f.path.as_str(), f)).collect())
            .unwrap_or_default();

        let mut files: Vec<FileDiff> = Vec::new();
        // All paths present in either side.
        let mut all_paths: Vec<&str> = cur.keys().chain(prev.keys()).copied().collect();
        all_paths.sort_unstable();
        all_paths.dedup();

        let mut total_delta = 0i64;
        for path in all_paths {
            let c = cur.get(path);
            let p = prev.get(path);
            let (status, words, delta) = match (c, p) {
                (Some(c), None) => ("added", c.words, c.words as i64),
                (None, Some(p)) => ("removed", 0, -(p.words as i64)),
                (Some(c), Some(p)) => {
                    if c.hash == p.hash {
                        ("unchanged", c.words, 0)
                    } else {
                        ("changed", c.words, c.words as i64 - p.words as i64)
                    }
                }
                (None, None) => unreachable!("path came from one of the maps"),
            };
            total_delta += delta;
            files.push(FileDiff {
                path: path.to_string(),
                status: status.to_string(),
                words,
                word_delta: delta,
            });
        }

        Ok(DiffSummary {
            id: commit.id.clone(),
            message: commit.message.clone(),
            parent: parent.map(|p| p.id.clone()),
            files,
            total_word_delta: total_delta,
        })
    }

    /// Restore a single `path` (if given) or the whole workspace to commit `id`.
    ///
    /// Restoring the whole workspace also deletes files that are not part of the
    /// target commit (byte-exact), matching a checkout. Restoring a single path
    /// only rewrites (or deletes) that one file.
    pub fn restore(&self, id: &CheckpointId, path: Option<&str>) -> Result<usize> {
        let idx = self.index_of(id)?;
        let commit = &self.commits[idx];

        match path {
            Some(path) => self.restore_one(commit, path),
            None => self.restore_all(commit),
        }
    }

    /// The word-count trail of a single chapter file across all commits.
    pub fn chapter_history(&self, path: &str) -> Vec<ChapterPoint> {
        let rel = normalize_rel(path);
        self.commits
            .iter()
            .map(|c| match c.files.iter().find(|f| f.path == rel) {
                Some(rec) => ChapterPoint {
                    commit: c.id.clone(),
                    ts: c.ts,
                    words: rec.words,
                    present: true,
                },
                None => ChapterPoint {
                    commit: c.id.clone(),
                    ts: c.ts,
                    words: 0,
                    present: false,
                },
            })
            .collect()
    }

    /// Number of commits (across all branches).
    pub fn len(&self) -> usize {
        self.commits.len()
    }

    /// Whether there are no commits.
    pub fn is_empty(&self) -> bool {
        self.commits.is_empty()
    }

    // ---------------------------------------------------------------------
    // branches / variants
    // ---------------------------------------------------------------------

    /// The currently checked-out branch name.
    pub fn current_branch(&self) -> &str {
        &self.state.current_branch
    }

    /// All known branch names, sorted.
    pub fn branches(&self) -> Vec<String> {
        let mut b = self.state.branches.clone();
        b.sort();
        b.dedup();
        b
    }

    /// Create a new branch `name` that forks from the current branch and switch
    /// to it. The new branch shares the existing history of the current branch;
    /// future commits diverge. Errors if the name already exists or is empty.
    pub fn branch(&mut self, name: &str) -> Result<()> {
        let name = name.trim();
        if name.is_empty() {
            return Err(CoreError::invalid_input("branch name must not be empty"));
        }
        if self.state.branches.iter().any(|b| b == name) {
            return Err(CoreError::conflict(format!(
                "branch {name:?} already exists"
            )));
        }
        let mut next = self.state.clone();
        next.branches.push(name.to_string());
        next.current_branch = name.to_string();
        self.persist_state(&next)?;
        self.state = next;
        Ok(())
    }

    /// Switch the working branch to an existing `name`. Errors if it is unknown.
    pub fn switch(&mut self, name: &str) -> Result<()> {
        let name = name.trim();
        if !self.state.branches.iter().any(|b| b == name) {
            return Err(CoreError::not_found(format!("no such branch {name:?}")));
        }
        let mut next = self.state.clone();
        next.current_branch = name.to_string();
        self.persist_state(&next)?;
        self.state = next;
        Ok(())
    }

    // ---------------------------------------------------------------------
    // line-level diff
    // ---------------------------------------------------------------------

    /// Structured line-level diff of `path` between revisions `rev_a` and
    /// `rev_b`. A missing file on either side is treated as empty content, so an
    /// added or removed file diffs cleanly. Uses an LCS so common lines are
    /// reported as context and only true edits are added/removed.
    pub fn diff_lines(
        &self,
        rev_a: &CheckpointId,
        rev_b: &CheckpointId,
        path: &str,
    ) -> Result<LineDiff> {
        // Validate both revisions exist (clear error if not).
        let _ = self.index_of(rev_a)?;
        let _ = self.index_of(rev_b)?;
        let rel = normalize_rel(path);

        let a_text = self.file_content_at(rev_a, &rel)?;
        let b_text = self.file_content_at(rev_b, &rel)?;

        let a_lines: Vec<&str> = split_lines(&a_text);
        let b_lines: Vec<&str> = split_lines(&b_text);

        let lines = lcs_diff(&a_lines, &b_lines);
        let added = lines.iter().filter(|l| l.kind == "added").count();
        let removed = lines.iter().filter(|l| l.kind == "removed").count();

        Ok(LineDiff {
            path: rel,
            rev_a: rev_a.clone(),
            rev_b: rev_b.clone(),
            lines,
            added,
            removed,
        })
    }

    // ---------------------------------------------------------------------
    // tags
    // ---------------------------------------------------------------------

    /// Attach a `label` to commit `rev`. A label is unique: re-tagging an
    /// existing label moves it to the new commit. Errors if the commit is
    /// unknown or the label is empty.
    pub fn tag(&mut self, rev: &CheckpointId, label: &str) -> Result<()> {
        let label = label.trim();
        if label.is_empty() {
            return Err(CoreError::invalid_input("tag label must not be empty"));
        }
        let _ = self.index_of(rev)?;
        let mut next = self.state.clone();
        if let Some(existing) = next.tags.iter_mut().find(|t| t.label == label) {
            existing.commit = rev.clone();
        } else {
            next.tags.push(Tag {
                label: label.to_string(),
                commit: rev.clone(),
            });
        }
        self.persist_state(&next)?;
        self.state = next;
        Ok(())
    }

    /// All tags, in insertion order.
    pub fn tags(&self) -> Vec<Tag> {
        self.state.tags.clone()
    }

    /// Resolve a tag label to its commit id, if it exists.
    pub fn resolve_tag(&self, label: &str) -> Option<CheckpointId> {
        self.state
            .tags
            .iter()
            .find(|t| t.label == label)
            .map(|t| t.commit.clone())
    }

    // ---------------------------------------------------------------------
    // internals
    // ---------------------------------------------------------------------

    fn index_of(&self, id: &CheckpointId) -> Result<usize> {
        self.commits
            .iter()
            .position(|c| &c.id == id)
            .ok_or_else(|| CoreError::not_found(format!("commit {id} not found")))
    }

    fn restore_one(&self, commit: &Commit, requested: &str) -> Result<usize> {
        let rel = self.normalize_restore_path(requested)?;
        let requested_key = path_comparison_key(&rel);
        let record = commit
            .files
            .iter()
            .find(|record| path_comparison_key(&record.path) == requested_key);
        let restored = record.is_some();
        let desired: Vec<&FileRecord> = record.into_iter().collect();
        let removals = if record.is_none() {
            vec![rel.as_str()]
        } else {
            Vec::new()
        };
        self.apply_restore(&desired, &removals)?;
        Ok(usize::from(restored))
    }

    fn restore_all(&self, commit: &Commit) -> Result<usize> {
        let desired: Vec<&FileRecord> = commit.files.iter().collect();
        let desired_paths: HashSet<String> = commit
            .files
            .iter()
            .map(|record| path_comparison_key(&record.path))
            .collect();
        let entries = self.collect_workspace_entries()?;
        let removals: Vec<&str> = entries
            .iter()
            .filter(|entry| !entry.is_dir)
            .filter(|entry| !desired_paths.contains(&path_comparison_key(&entry.rel)))
            .map(|entry| entry.rel.as_str())
            .collect();
        self.apply_restore(&desired, &removals)?;
        Ok(commit.files.len())
    }

    fn normalize_restore_path(&self, requested: &str) -> Result<String> {
        let separators_normalized = normalize_user_separators(requested);
        // Resolve once to reject lexical escapes and links that leave the jail,
        // but keep the lexical relative path. Using the canonical result here
        // would silently turn an in-workspace symlink alias into its target.
        let resolved = self.jail.resolve(&separators_normalized)?;
        if self
            .jail
            .relative(&resolved)
            .is_some_and(|path| path.split('/').any(is_reserved_path_component))
        {
            return Err(CoreError::security(format!(
                "restore access to internal workspace state is blocked: {requested}"
            )));
        }
        let rel = lexical_workspace_relative(
            &self.jail,
            &self.workspace_root,
            Path::new(&separators_normalized),
        )
        .ok_or_else(|| {
            CoreError::sandbox(format!(
                "restore path is outside the workspace: {requested:?}"
            ))
        })?;
        if rel.is_empty() {
            return Err(CoreError::invalid_input(
                "restore path must identify a workspace file",
            ));
        }
        validate_user_restore_path(&rel)?;
        Ok(rel)
    }

    fn apply_restore(&self, desired: &[&FileRecord], removals: &[&str]) -> Result<()> {
        validate_file_record_refs(desired)?;

        // Read every object before creating the transaction or touching the
        // workspace. Corruption and missing blobs therefore fail cleanly.
        let mut blobs = Vec::with_capacity(desired.len());
        for record in desired {
            blobs.push((record.path.as_str(), self.read_blob(&record.hash)?));
        }

        // Reject links in any existing destination component, even links that
        // remain inside the workspace. Restores replace the named path itself;
        // they never write through a filesystem alias.
        let mut backup_roots = Vec::new();
        for record in desired {
            if let Some(conflict) = self.preflight_destination(&record.path)? {
                backup_roots.push(conflict);
            }
        }
        for rel in removals {
            validate_stored_path(rel)?;
            if self.preflight_removal(rel)? {
                backup_roots.push((*rel).to_string());
            }
        }
        minimize_restore_roots(&mut backup_roots);
        for rel in &backup_roots {
            let path = self.workspace_path(rel);
            let is_directory = fs::symlink_metadata(&path)
                .map(|metadata| metadata.is_dir())
                .map_err(|error| {
                    CoreError::from(error)
                        .with_context(format!("inspecting restore backup root {rel}"))
                })?;
            if is_directory && self.contains_reserved_descendant(&path)? {
                return Err(CoreError::security(format!(
                    "restore would replace protected state below {rel:?}"
                )));
            }
        }

        ensure_store_directory(&self.vcs_dir, ".na-vcs")?;
        let transaction = self.vcs_dir.join(na_common::next_id("restore-transaction"));
        fs::create_dir(&transaction).map_err(|error| {
            CoreError::from(error).with_context("creating VCS restore transaction")
        })?;
        let stage = transaction.join("stage");
        let backup = transaction.join("backup");
        let setup_result = (|| -> Result<()> {
            fs::create_dir(&stage).map_err(|error| {
                CoreError::from(error).with_context("creating VCS restore staging directory")
            })?;
            fs::create_dir(&backup).map_err(|error| {
                CoreError::from(error).with_context("creating VCS restore backup directory")
            })?;
            for (rel, blob) in &blobs {
                let staged = join_stored_path(&stage, rel);
                if let Some(parent) = staged.parent() {
                    fs::create_dir_all(parent).map_err(|error| {
                        CoreError::from(error)
                            .with_context(format!("staging restore directories for {rel}"))
                    })?;
                }
                atomic_write(&staged, blob, rel)?;
            }
            Ok(())
        })();
        if let Err(error) = setup_result {
            let _ = fs::remove_dir_all(&transaction);
            return Err(error);
        }

        let mut moved_backups: Vec<String> = Vec::new();
        let mut installed: Vec<String> = Vec::new();
        let mut created_dirs: Vec<PathBuf> = Vec::new();
        let apply_result = (|| -> Result<()> {
            for rel in &backup_roots {
                let source = self.workspace_path(rel);
                let destination = join_stored_path(&backup, rel);
                if let Some(parent) = destination.parent() {
                    fs::create_dir_all(parent).map_err(|error| {
                        CoreError::from(error)
                            .with_context(format!("creating restore backup directory for {rel}"))
                    })?;
                }
                fs::rename(&source, &destination).map_err(|error| {
                    CoreError::from(error).with_context(format!("backing up {rel} during restore"))
                })?;
                moved_backups.push(rel.clone());
            }

            for record in desired {
                self.ensure_destination_parents(&record.path, &mut created_dirs)?;
                // Recheck after the backups and directory creation so a static
                // destination link can never be followed during installation.
                if self.preflight_destination(&record.path)?.is_some() {
                    return Err(CoreError::conflict(format!(
                        "restore destination changed while applying {:?}",
                        record.path
                    )));
                }
                let source = join_stored_path(&stage, &record.path);
                let destination = self.workspace_path(&record.path);
                fs::rename(&source, &destination).map_err(|error| {
                    CoreError::from(error)
                        .with_context(format!("installing {} during restore", record.path))
                })?;
                installed.push(record.path.clone());
            }
            Ok(())
        })();

        if let Err(error) = apply_result {
            let rollback = self.rollback_restore(
                &backup,
                &mut installed,
                &mut moved_backups,
                &mut created_dirs,
            );
            if rollback.is_ok() {
                let _ = fs::remove_dir_all(&transaction);
                return Err(error);
            }
            return Err(error.with_context(format!(
                "restore rollback also failed; preserved recovery data at {}: {}",
                transaction.display(),
                rollback.unwrap_err()
            )));
        }

        fs::remove_dir_all(&transaction).map_err(|error| {
            CoreError::from(error).with_context(format!(
                "removing completed restore transaction {}",
                transaction.display()
            ))
        })?;
        Ok(())
    }

    fn preflight_destination(&self, rel: &str) -> Result<Option<String>> {
        validate_stored_path(rel)?;
        let components: Vec<&str> = rel.split('/').collect();
        let mut current = self.workspace_root.clone();
        let mut current_rel = String::new();
        for (index, component) in components.iter().enumerate() {
            current.push(component);
            if !current_rel.is_empty() {
                current_rel.push('/');
            }
            current_rel.push_str(component);
            match fs::symlink_metadata(&current) {
                Ok(metadata) => {
                    if metadata.file_type().is_symlink() {
                        return Err(destination_symlink_error(&current_rel));
                    }
                    if index + 1 == components.len() || !metadata.is_dir() {
                        return Ok(Some(current_rel));
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(error) => {
                    return Err(CoreError::from(error).with_context(format!(
                        "inspecting restore destination {}",
                        current.display()
                    )))
                }
            }
        }
        Ok(None)
    }

    fn preflight_removal(&self, rel: &str) -> Result<bool> {
        let components: Vec<&str> = rel.split('/').collect();
        let mut current = self.workspace_root.clone();
        for (index, component) in components.iter().enumerate() {
            current.push(component);
            match fs::symlink_metadata(&current) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err(destination_symlink_error(rel))
                }
                Ok(_) if index + 1 == components.len() => return Ok(true),
                Ok(metadata) if metadata.is_dir() => {}
                Ok(_) => return Ok(false),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
                Err(error) => {
                    return Err(CoreError::from(error)
                        .with_context(format!("inspecting restore removal {}", current.display())))
                }
            }
        }
        Ok(false)
    }

    fn ensure_destination_parents(&self, rel: &str, created: &mut Vec<PathBuf>) -> Result<()> {
        let components: Vec<&str> = rel.split('/').collect();
        let mut current = self.workspace_root.clone();
        for component in &components[..components.len().saturating_sub(1)] {
            current.push(component);
            match fs::symlink_metadata(&current) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err(destination_symlink_error(rel))
                }
                Ok(metadata) if metadata.is_dir() => {}
                Ok(_) => {
                    return Err(CoreError::conflict(format!(
                        "non-directory restore ancestor {}",
                        current.display()
                    )))
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    fs::create_dir(&current).map_err(|error| {
                        CoreError::from(error).with_context(format!(
                            "creating restore destination directory {}",
                            current.display()
                        ))
                    })?;
                    created.push(current.clone());
                }
                Err(error) => {
                    return Err(CoreError::from(error).with_context(format!(
                        "inspecting restore destination directory {}",
                        current.display()
                    )))
                }
            }
        }
        Ok(())
    }

    fn rollback_restore(
        &self,
        backup: &Path,
        installed: &mut Vec<String>,
        moved_backups: &mut Vec<String>,
        created_dirs: &mut Vec<PathBuf>,
    ) -> Result<()> {
        let mut errors = Vec::new();
        while let Some(rel) = installed.pop() {
            let path = self.workspace_path(&rel);
            if let Err(error) = fs::remove_file(&path) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    errors.push(format!("removing installed {rel}: {error}"));
                }
            }
        }
        while let Some(path) = created_dirs.pop() {
            if let Err(error) = fs::remove_dir(&path) {
                // ErrorKind::DirectoryNotEmpty is newer than the workspace's
                // Rust 1.80 MSRV. Confirm the condition from the directory
                // contents so permission and other I/O failures still surface.
                let directory_not_empty = fs::read_dir(&path)
                    .ok()
                    .and_then(|mut entries| entries.next())
                    .is_some();
                if error.kind() != std::io::ErrorKind::NotFound && !directory_not_empty {
                    errors.push(format!("removing created {}: {error}", path.display()));
                }
            }
        }
        while let Some(rel) = moved_backups.pop() {
            let source = join_stored_path(backup, &rel);
            let destination = self.workspace_path(&rel);
            if let Some(parent) = destination.parent() {
                if let Err(error) = fs::create_dir_all(parent) {
                    errors.push(format!("recreating parent for {rel}: {error}"));
                    continue;
                }
            }
            if let Err(error) = fs::rename(&source, &destination) {
                errors.push(format!("restoring backup for {rel}: {error}"));
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(CoreError::internal(errors.join("; ")))
        }
    }

    fn workspace_path(&self, rel: &str) -> PathBuf {
        join_stored_path(&self.workspace_root, rel)
    }

    fn collect_workspace_entries(&self) -> Result<Vec<WorkspaceEntry>> {
        let mut entries = Vec::new();
        self.collect_entries_from(&self.workspace_root, &mut entries)?;
        Ok(entries)
    }

    fn collect_entries_from(&self, dir: &Path, out: &mut Vec<WorkspaceEntry>) -> Result<()> {
        let read = fs::read_dir(dir).map_err(|error| {
            CoreError::from(error).with_context(format!("reading {}", dir.display()))
        })?;
        for entry in read {
            let entry = entry.map_err(CoreError::from)?;
            let path = entry.path();
            if self.is_ignored(&path) {
                continue;
            }
            let file_type = entry.file_type().map_err(CoreError::from)?;
            let rel = self.rel_path(&path)?;
            out.push(WorkspaceEntry {
                rel,
                is_dir: file_type.is_dir(),
            });
            if file_type.is_dir() {
                self.collect_entries_from(&path, out)?;
            }
        }
        Ok(())
    }

    fn contains_reserved_descendant(&self, dir: &Path) -> Result<bool> {
        for entry in fs::read_dir(dir).map_err(|error| {
            CoreError::from(error).with_context(format!("inspecting {}", dir.display()))
        })? {
            let entry = entry.map_err(CoreError::from)?;
            let name = entry.file_name();
            if name.to_str().is_some_and(is_reserved_path_component) {
                return Ok(true);
            }
            let file_type = entry.file_type().map_err(CoreError::from)?;
            if file_type.is_dir() && self.contains_reserved_descendant(&entry.path())? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn snapshot_dir(&self, dir: &Path, out: &mut Vec<FileRecord>) -> Result<()> {
        let read = fs::read_dir(dir)
            .map_err(|e| CoreError::from(e).with_context(format!("reading {}", dir.display())))?;
        for entry in read {
            let entry = entry.map_err(CoreError::from)?;
            let path = entry.path();
            if self.is_ignored(&path) {
                continue;
            }
            let ft = entry.file_type().map_err(CoreError::from)?;
            if ft.is_dir() {
                self.snapshot_dir(&path, out)?;
            } else if ft.is_file() {
                let bytes = fs::read(&path).map_err(|e| {
                    CoreError::from(e).with_context(format!("reading {}", path.display()))
                })?;
                let hash = content_hash(&bytes);
                self.write_blob_if_absent(&hash, &bytes)?;
                let words = match std::str::from_utf8(&bytes) {
                    Ok(s) => count_words(s),
                    Err(_) => 0, // binary file: zero words
                };
                let rel = self.rel_path(&path)?;
                out.push(FileRecord {
                    path: rel,
                    hash,
                    words,
                });
            }
        }
        Ok(())
    }

    /// Ignore the VCS dir, the `.na` state dir, and `.git`.
    fn is_ignored(&self, path: &Path) -> bool {
        if path.starts_with(&self.vcs_dir) {
            return true;
        }
        path.file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| {
                name.eq_ignore_ascii_case(VCS_DIR)
                    || name.eq_ignore_ascii_case(".na")
                    || name.eq_ignore_ascii_case(".git")
            })
    }

    fn rel_path(&self, path: &Path) -> Result<String> {
        let rel = path.strip_prefix(&self.workspace_root).map_err(|_| {
            CoreError::internal(format!("{} not under workspace root", path.display()))
        })?;
        let mut parts = Vec::new();
        for comp in rel.components() {
            match comp {
                Component::Normal(os) => parts.push(
                    os.to_str()
                        .ok_or_else(|| {
                            CoreError::invalid_input(format!(
                                "workspace path is not valid UTF-8: {}",
                                path.display()
                            ))
                        })?
                        .to_string(),
                ),
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

    fn append_commit(&self, commit: &Commit) -> Result<()> {
        ensure_metadata_file_or_missing(&self.commits_path, ".na-vcs/commits.jsonl")?;
        let mut bytes = Vec::new();
        for existing in &self.commits {
            serde_json::to_writer(&mut bytes, existing)?;
            bytes.push(b'\n');
        }
        serde_json::to_writer(&mut bytes, commit)?;
        bytes.push(b'\n');
        atomic_write(&self.commits_path, &bytes, ".na-vcs/commits.jsonl")
    }

    fn persist_state(&self, state: &VcsState) -> Result<()> {
        validate_state(state, &self.commits)?;
        ensure_metadata_file_or_missing(&self.state_path, ".na-vcs/state.json")?;
        let json = serde_json::to_string_pretty(state)?;
        atomic_write(&self.state_path, json.as_bytes(), ".na-vcs/state.json")
    }

    /// The UTF-8 content of `rel` (already normalized) at commit `id`. A file
    /// absent from that commit reads as the empty string. Binary blobs are
    /// decoded lossily so a diff still renders.
    fn file_content_at(&self, id: &CheckpointId, rel: &str) -> Result<String> {
        let idx = self.index_of(id)?;
        let commit = &self.commits[idx];
        match commit.files.iter().find(|f| f.path == rel) {
            Some(rec) => {
                let bytes = self.read_blob(&rec.hash)?;
                Ok(String::from_utf8_lossy(&bytes).into_owned())
            }
            None => Ok(String::new()),
        }
    }
}

/// Normalize a user path to forward-slash, stripping leading `./` and slashes.
fn normalize_rel(rel: &str) -> String {
    let mut s = normalize_user_separators(rel);
    while let Some(stripped) = s.strip_prefix("./") {
        s = stripped.to_string();
    }
    s.trim_start_matches('/').to_string()
}

/// Load commits from the JSON-Lines log (missing file => empty), sorted by time.
fn load_commits(path: &Path) -> Result<Vec<Commit>> {
    ensure_metadata_file_or_missing(path, ".na-vcs/commits.jsonl")?;
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(CoreError::from(error).with_context("reading commits.jsonl"));
        }
    };
    let terminated = bytes.last().map_or(true, |byte| *byte == b'\n');
    let mut lines: Vec<&[u8]> = bytes.split(|byte| *byte == b'\n').collect();
    if terminated {
        lines.pop();
    }
    let line_count = lines.len();
    let mut out = Vec::new();
    let mut ids = HashSet::new();
    for (lineno, raw_line) in lines.into_iter().enumerate() {
        let line = match std::str::from_utf8(raw_line) {
            Ok(line) => line,
            Err(_) if !terminated && lineno + 1 == line_count => break,
            Err(error) => {
                return Err(CoreError::new(
                    na_common::ErrorKind::Serialization,
                    format!("commit log line {} is not UTF-8: {error}", lineno + 1),
                ))
            }
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let commit: Commit = match serde_json::from_str(trimmed) {
            Ok(commit) => commit,
            // Older versions appended JSON and its newline separately. A
            // process crash could therefore leave one partial final record;
            // that record was never committed and can be discarded safely.
            Err(_) if !terminated && lineno + 1 == line_count => break,
            Err(error) => {
                return Err(CoreError::from(error)
                    .with_context(format!("parsing commit at line {}", lineno + 1)))
            }
        };
        if commit.id.0.is_empty() || !ids.insert(commit.id.0.clone()) {
            return Err(CoreError::new(
                na_common::ErrorKind::Serialization,
                format!("invalid or duplicate commit id at line {}", lineno + 1),
            ));
        }
        if commit.branch.trim().is_empty() {
            return Err(CoreError::new(
                na_common::ErrorKind::Serialization,
                format!("empty commit branch at line {}", lineno + 1),
            ));
        }
        validate_file_records(&commit.files)
            .map_err(|error| error.with_context(format!("commit log line {}", lineno + 1)))?;
        out.push(commit);
    }
    out.sort_by_key(|c| c.ts);
    Ok(out)
}

fn validate_stored_path(path: &str) -> Result<()> {
    if path.is_empty()
        || path.split('/').any(|part| {
            part.is_empty()
                || part == "."
                || part == ".."
                || is_reserved_path_component(part)
                || part.contains('\0')
                || !is_valid_platform_component(part)
        })
    {
        return Err(invalid_stored_path(path));
    }
    Ok(())
}

fn invalid_stored_path(path: &str) -> CoreError {
    CoreError::new(
        na_common::ErrorKind::Serialization,
        format!("invalid commit path {path:?}"),
    )
}

fn validate_user_restore_path(path: &str) -> Result<()> {
    if path.split('/').any(is_reserved_path_component) {
        return Err(CoreError::security(format!(
            "restore access to internal workspace state is blocked: {path}"
        )));
    }
    validate_stored_path(path).map_err(|_| {
        CoreError::invalid_input(format!("invalid workspace-relative restore path {path:?}"))
    })
}

fn is_reserved_path_component(component: &str) -> bool {
    let component = path_component_comparison_key(component);
    matches!(component.as_str(), ".na" | ".na-vcs" | ".git")
}

fn validate_file_records(files: &[FileRecord]) -> Result<()> {
    let refs: Vec<&FileRecord> = files.iter().collect();
    validate_file_record_refs(&refs)
}

fn validate_file_record_refs(files: &[&FileRecord]) -> Result<()> {
    let mut seen = HashSet::new();
    for record in files {
        validate_stored_path(&record.path)?;
        validate_content_hash(&record.hash)?;
        let key = path_comparison_key(&record.path);
        if !seen.insert(key) {
            return Err(CoreError::new(
                na_common::ErrorKind::Serialization,
                format!("duplicate or aliased commit path {:?}", record.path),
            ));
        }
    }

    for record in files {
        let key = path_comparison_key(&record.path);
        for (index, _) in key.match_indices('/') {
            let ancestor = &key[..index];
            if seen.contains(ancestor) {
                return Err(CoreError::new(
                    na_common::ErrorKind::Serialization,
                    format!(
                        "commit path {:?} conflicts with its file ancestor {ancestor:?}",
                        record.path
                    ),
                ));
            }
        }
    }
    Ok(())
}

#[cfg(windows)]
fn path_comparison_key(path: &str) -> String {
    path.split('/')
        .map(path_component_comparison_key)
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(not(windows))]
fn path_comparison_key(path: &str) -> String {
    path.to_string()
}

#[cfg(windows)]
fn path_component_comparison_key(component: &str) -> String {
    component.trim_end_matches([' ', '.']).to_lowercase()
}

#[cfg(not(windows))]
fn path_component_comparison_key(component: &str) -> String {
    component.to_ascii_lowercase()
}

#[cfg(windows)]
fn is_valid_platform_component(component: &str) -> bool {
    if component.ends_with([' ', '.'])
        || component.chars().any(|character| {
            character <= '\u{1f}'
                || matches!(character, '<' | '>' | ':' | '"' | '\\' | '|' | '?' | '*')
        })
    {
        return false;
    }
    let stem = component
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    !matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$")
        && !matches!(
            stem.as_str(),
            "COM1" | "COM2" | "COM3" | "COM4" | "COM5" | "COM6" | "COM7" | "COM8" | "COM9"
        )
        && !matches!(
            stem.as_str(),
            "LPT1" | "LPT2" | "LPT3" | "LPT4" | "LPT5" | "LPT6" | "LPT7" | "LPT8" | "LPT9"
        )
}

#[cfg(not(windows))]
fn is_valid_platform_component(_component: &str) -> bool {
    true
}

#[cfg(windows)]
fn normalize_user_separators(path: &str) -> String {
    path.replace('\\', "/")
}

#[cfg(not(windows))]
fn normalize_user_separators(path: &str) -> String {
    path.to_string()
}

#[cfg(windows)]
fn lexical_workspace_relative(jail: &PathJail, root: &Path, path: &Path) -> Option<String> {
    if path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::Prefix(_)))
    {
        let root_key = windows_absolute_key(root);
        let path_key = windows_absolute_key(path);
        let suffix = path_key.strip_prefix(&root_key)?;
        if !suffix.is_empty() && !suffix.starts_with('/') {
            return None;
        }
        normalize_lexical_relative(suffix.trim_start_matches('/'))
    } else {
        jail.relative(path)
    }
}

#[cfg(not(windows))]
fn lexical_workspace_relative(jail: &PathJail, _root: &Path, path: &Path) -> Option<String> {
    jail.relative(path)
}

#[cfg(windows)]
fn windows_absolute_key(path: &Path) -> String {
    let raw = path.to_string_lossy().replace('\\', "/");
    let raw = raw
        .strip_prefix("//?/UNC/")
        .map(|rest| format!("//{rest}"))
        .or_else(|| raw.strip_prefix("//?/").map(ToOwned::to_owned))
        .unwrap_or(raw);
    raw.trim_end_matches('/').to_lowercase()
}

#[cfg(windows)]
fn normalize_lexical_relative(path: &str) -> Option<String> {
    let mut components: Vec<&str> = Vec::new();
    for component in path.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                components.pop()?;
            }
            component => components.push(component),
        }
    }
    Some(components.join("/"))
}

fn join_stored_path(base: &Path, rel: &str) -> PathBuf {
    let mut path = base.to_path_buf();
    for component in rel.split('/') {
        path.push(component);
    }
    path
}

fn minimize_restore_roots(roots: &mut Vec<String>) {
    roots.sort_unstable_by(|left, right| {
        left.split('/')
            .count()
            .cmp(&right.split('/').count())
            .then_with(|| path_comparison_key(left).cmp(&path_comparison_key(right)))
    });
    let mut kept: Vec<String> = Vec::new();
    for root in roots.drain(..) {
        let key = path_comparison_key(&root);
        let covered = kept.iter().any(|ancestor| {
            let ancestor = path_comparison_key(ancestor);
            key == ancestor
                || key
                    .strip_prefix(&ancestor)
                    .is_some_and(|suffix| suffix.starts_with('/'))
        });
        if !covered {
            kept.push(root);
        }
    }
    *roots = kept;
}

fn destination_symlink_error(path: &str) -> CoreError {
    CoreError::sandbox(format!(
        "restore destination contains a filesystem link: {path:?}"
    ))
}

fn ensure_store_directory(path: &Path, description: &str) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => Err(
            CoreError::sandbox(format!("{description} is not a regular directory")),
        ),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(path).map_err(|error| {
                CoreError::from(error).with_context(format!("creating {description}"))
            })
        }
        Err(error) => {
            Err(CoreError::from(error).with_context(format!("reading metadata for {description}")))
        }
    }
}

fn ensure_metadata_file_or_missing(path: &Path, description: &str) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => Err(
            CoreError::sandbox(format!("{description} is not a regular file")),
        ),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => {
            Err(CoreError::from(error).with_context(format!("reading metadata for {description}")))
        }
    }
}

fn validate_state(state: &VcsState, commits: &[Commit]) -> Result<()> {
    if state.current_branch.trim().is_empty() {
        return Err(invalid_state("current branch must not be empty"));
    }
    let mut branches = HashSet::new();
    for branch in &state.branches {
        if branch.trim().is_empty() || !branches.insert(branch.as_str()) {
            return Err(invalid_state("branches must be non-empty and unique"));
        }
    }
    let commit_ids: HashSet<&str> = commits.iter().map(|commit| commit.id.as_str()).collect();
    let mut labels = HashSet::new();
    for tag in &state.tags {
        if tag.label.trim().is_empty() || !labels.insert(tag.label.as_str()) {
            return Err(invalid_state("tag labels must be non-empty and unique"));
        }
        if !commit_ids.contains(tag.commit.as_str()) {
            return Err(invalid_state(format!(
                "tag {:?} refers to unknown commit {}",
                tag.label, tag.commit
            )));
        }
    }
    Ok(())
}

fn invalid_state(message: impl Into<String>) -> CoreError {
    CoreError::new(na_common::ErrorKind::Serialization, message)
        .with_context("invalid .na-vcs/state.json")
}

/// Load the branch/tag state (missing file => defaults).
fn load_state(path: &Path) -> Result<VcsState> {
    ensure_metadata_file_or_missing(path, ".na-vcs/state.json")?;
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(VcsState::default())
        }
        Err(error) => return Err(CoreError::from(error).with_context("reading .na-vcs/state.json")),
    };
    if text.trim().is_empty() {
        return Ok(VcsState::default());
    }
    serde_json::from_str(&text)
        .map_err(|e| CoreError::from(e).with_context("parsing .na-vcs/state.json"))
}

/// Split text into lines, preserving content but dropping the line terminators.
/// A trailing newline does not produce a spurious empty final line.
fn split_lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut lines: Vec<&str> = text.split('\n').collect();
    // `"a\n".split('\n')` -> ["a", ""]; drop the trailing empty element.
    if let Some(last) = lines.last() {
        if last.is_empty() {
            lines.pop();
        }
    }
    lines
}

/// Produce a line-level diff of `a` vs `b` using a longest-common-subsequence
/// backtrace. Common lines are emitted as `context`, lines only in `a` as
/// `removed`, and lines only in `b` as `added`, in source order.
fn lcs_diff(a: &[&str], b: &[&str]) -> Vec<DiffLine> {
    let n = a.len();
    let m = b.len();

    // DP table of LCS lengths: (n+1) x (m+1).
    let mut dp = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if a[i] == b[j] {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }

    // Backtrace to build the edit script.
    let mut out = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    while i < n && j < m {
        if a[i] == b[j] {
            out.push(DiffLine {
                kind: "context".to_string(),
                a_line: Some(i + 1),
                b_line: Some(j + 1),
                text: a[i].to_string(),
            });
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            out.push(DiffLine {
                kind: "removed".to_string(),
                a_line: Some(i + 1),
                b_line: None,
                text: a[i].to_string(),
            });
            i += 1;
        } else {
            out.push(DiffLine {
                kind: "added".to_string(),
                a_line: None,
                b_line: Some(j + 1),
                text: b[j].to_string(),
            });
            j += 1;
        }
    }
    while i < n {
        out.push(DiffLine {
            kind: "removed".to_string(),
            a_line: Some(i + 1),
            b_line: None,
            text: a[i].to_string(),
        });
        i += 1;
    }
    while j < m {
        out.push(DiffLine {
            kind: "added".to_string(),
            a_line: None,
            b_line: Some(j + 1),
            text: b[j].to_string(),
        });
        j += 1;
    }
    out
}

// =========================================================================
// Tools
// =========================================================================

/// Commit the current workspace as a new fiction-VCS snapshot.
#[derive(Debug, Clone, Copy, Default)]
pub struct GitCommitTool;

impl Tool for GitCommitTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "vcs_commit",
            "Snapshot the current manuscript as a versioned commit with a message. \
             Records per-file word counts.",
            json!({
                "type": "object",
                "required": ["message"],
                "properties": { "message": { "type": "string", "minLength": 1 } },
                "additionalProperties": false
            }),
            vec![Capability::GitWrite],
            true,
        )
    }

    fn execute<'a>(
        &'a self,
        args: Json,
        ctx: &'a ToolContext,
    ) -> BoxFuture<'a, Result<ToolResult>> {
        Box::pin(async move {
            let message = args
                .get("message")
                .and_then(Json::as_str)
                .ok_or_else(|| CoreError::invalid_input("missing string argument \"message\""))?;
            let mut vcs = FictionVcs::open(ctx.jail.root())?;
            let id = vcs.commit(message)?;
            let total = vcs.log().last().map(|e| e.total_words).unwrap_or(0);
            Ok(ToolResult::success(
                format!("committed {id} ({total} words)"),
                json!({ "id": id.as_str(), "total_words": total }),
            )
            .with_summary(format!("commit {}", id.as_str())))
        })
    }
}

/// Show the commit history with word totals and deltas.
#[derive(Debug, Clone, Copy, Default)]
pub struct GitLogTool;

impl Tool for GitLogTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "vcs_log",
            "Show the manuscript commit history: id, message, total words, and word delta.",
            json!({ "type": "object", "properties": {}, "additionalProperties": false }),
            vec![],
            false,
        )
    }

    fn execute<'a>(
        &'a self,
        _args: Json,
        ctx: &'a ToolContext,
    ) -> BoxFuture<'a, Result<ToolResult>> {
        Box::pin(async move {
            let vcs = FictionVcs::open(ctx.jail.root())?;
            let log = vcs.log();
            let mut text = String::new();
            for e in &log {
                let sign = if e.word_delta >= 0 { "+" } else { "" };
                text.push_str(&format!(
                    "{} | {} words ({sign}{}) | {}\n",
                    e.id.as_str(),
                    e.total_words,
                    e.word_delta,
                    e.message
                ));
            }
            if text.is_empty() {
                text.push_str("(no commits yet)");
            }
            let value = serde_json::to_value(&log)?;
            Ok(
                ToolResult::success(text, json!({ "commits": value, "count": log.len() }))
                    .with_summary(format!("{} commit(s)", log.len())),
            )
        })
    }
}

/// Show the per-file word diff of a commit versus its predecessor.
#[derive(Debug, Clone, Copy, Default)]
pub struct GitDiffTool;

impl Tool for GitDiffTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "vcs_diff",
            "Summarize what changed in a commit (added/removed/changed files and word deltas).",
            json!({
                "type": "object",
                "required": ["id"],
                "properties": { "id": { "type": "string", "minLength": 1 } },
                "additionalProperties": false
            }),
            vec![],
            false,
        )
    }

    fn execute<'a>(
        &'a self,
        args: Json,
        ctx: &'a ToolContext,
    ) -> BoxFuture<'a, Result<ToolResult>> {
        Box::pin(async move {
            let id = args
                .get("id")
                .and_then(Json::as_str)
                .ok_or_else(|| CoreError::invalid_input("missing string argument \"id\""))?;
            let vcs = FictionVcs::open(ctx.jail.root())?;
            let summary = vcs.diff(&CheckpointId::from_existing(id))?;
            let mut text = String::new();
            for f in &summary.files {
                if f.status == "unchanged" {
                    continue;
                }
                let sign = if f.word_delta >= 0 { "+" } else { "" };
                text.push_str(&format!(
                    "{:9} {} ({sign}{} words)\n",
                    f.status, f.path, f.word_delta
                ));
            }
            text.push_str(&format!("net: {} words", summary.total_word_delta));
            let value = serde_json::to_value(&summary)?;
            Ok(ToolResult::success(text, value)
                .with_summary(format!("diff {id}: {} words", summary.total_word_delta)))
        })
    }
}

/// Restore a file or the whole workspace to a commit.
#[derive(Debug, Clone, Copy, Default)]
pub struct GitRestoreTool;

impl Tool for GitRestoreTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "vcs_restore",
            "Restore the manuscript (or a single file) to a previous commit.",
            json!({
                "type": "object",
                "required": ["id"],
                "properties": {
                    "id": { "type": "string", "minLength": 1 },
                    "path": { "type": "string",
                        "description": "Restore only this file; omit to restore everything." }
                },
                "additionalProperties": false
            }),
            vec![Capability::GitWrite],
            true,
        )
    }

    fn execute<'a>(
        &'a self,
        args: Json,
        ctx: &'a ToolContext,
    ) -> BoxFuture<'a, Result<ToolResult>> {
        Box::pin(async move {
            let id = args
                .get("id")
                .and_then(Json::as_str)
                .ok_or_else(|| CoreError::invalid_input("missing string argument \"id\""))?;
            let path = args.get("path").and_then(Json::as_str);
            let vcs = FictionVcs::open(ctx.jail.root())?;
            let restored = vcs.restore(&CheckpointId::from_existing(id), path)?;
            let scope = path
                .map(|p| p.to_string())
                .unwrap_or_else(|| "all files".to_string());
            Ok(ToolResult::success(
                format!("restored {scope} to {id} ({restored} file(s))"),
                json!({ "id": id, "restored": restored, "path": path }),
            )
            .with_summary(format!("restored to {id}")))
        })
    }
}

/// Manage manuscript branches (alternate plot lines): create, switch, or list.
#[derive(Debug, Clone, Copy, Default)]
pub struct GitBranchTool;

impl Tool for GitBranchTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "vcs_branch",
            "Manage manuscript branches for exploring alternate plot lines. \
             action=create forks a new branch from the current one and switches to it; \
             action=switch checks out an existing branch; action=list shows all branches \
             and the current one.",
            json!({
                "type": "object",
                "required": ["action"],
                "properties": {
                    "action": { "type": "string", "enum": ["create", "switch", "list"] },
                    "name": { "type": "string",
                        "description": "Branch name (required for create/switch)." }
                },
                "additionalProperties": false
            }),
            vec![Capability::GitWrite],
            true,
        )
    }

    fn execute<'a>(
        &'a self,
        args: Json,
        ctx: &'a ToolContext,
    ) -> BoxFuture<'a, Result<ToolResult>> {
        Box::pin(async move {
            let action = args
                .get("action")
                .and_then(Json::as_str)
                .ok_or_else(|| CoreError::invalid_input("missing string argument \"action\""))?;
            let mut vcs = FictionVcs::open(ctx.jail.root())?;

            match action {
                "create" => {
                    let name = require_name(&args)?;
                    vcs.branch(name)?;
                    Ok(ToolResult::success(
                        format!("created and switched to branch {name:?}"),
                        json!({
                            "action": "create",
                            "current": vcs.current_branch(),
                            "branches": vcs.branches(),
                        }),
                    )
                    .with_summary(format!("branch {name}")))
                }
                "switch" => {
                    let name = require_name(&args)?;
                    vcs.switch(name)?;
                    Ok(ToolResult::success(
                        format!("switched to branch {name:?}"),
                        json!({
                            "action": "switch",
                            "current": vcs.current_branch(),
                            "branches": vcs.branches(),
                        }),
                    )
                    .with_summary(format!("switch {name}")))
                }
                "list" => {
                    let branches = vcs.branches();
                    let current = vcs.current_branch().to_string();
                    let mut text = String::new();
                    for b in &branches {
                        let marker = if *b == current { "* " } else { "  " };
                        text.push_str(&format!("{marker}{b}\n"));
                    }
                    if text.is_empty() {
                        text.push_str("(no branches)");
                    }
                    Ok(ToolResult::success(
                        text,
                        json!({ "action": "list", "current": current, "branches": branches }),
                    )
                    .with_summary(format!("{} branch(es)", branches.len())))
                }
                other => Err(CoreError::invalid_input(format!(
                    "unknown branch action {other:?} (expected create/switch/list)"
                ))),
            }
        })
    }
}

/// Extract a required, non-empty `name` argument for branch create/switch.
fn require_name(args: &Json) -> Result<&str> {
    args.get("name")
        .and_then(Json::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| {
            CoreError::invalid_input("this action requires a non-empty \"name\" argument")
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool::ToolContextBuilder;

    fn temp_root(tag: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("na_tools_vcs_{}_{}", tag, na_common::next_id("t")));
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn count_words_cjk_and_ascii() {
        assert_eq!(count_words("Hello world"), 2);
        assert_eq!(count_words("你好世界"), 4); // 4 CJK chars
        assert_eq!(count_words("Hello 世界"), 3); // 1 + 2
        assert_eq!(count_words("  spaced   out  "), 2);
        assert_eq!(count_words(""), 0);
        assert_eq!(count_words("林惊羽 said hi"), 3 + 2); // 3 cjk + 2 words
    }

    #[test]
    fn commit_log_restore_and_delta() {
        let root = temp_root("clr");
        fs::write(root.join("ch1.md"), "第一章 林惊羽出场").unwrap(); // 7 cjk words
        let mut vcs = FictionVcs::open(&root).unwrap();
        let c1 = vcs.commit("初稿").unwrap();
        let log1 = vcs.log();
        assert_eq!(log1.len(), 1);
        let words1 = log1[0].total_words;
        assert!(words1 > 0);
        assert_eq!(log1[0].word_delta, words1 as i64);

        // Grow the chapter.
        fs::write(root.join("ch1.md"), "第一章 林惊羽出场 他拔出了霜寒剑").unwrap();
        let c2 = vcs.commit("扩写").unwrap();
        let log2 = vcs.log();
        assert_eq!(log2.len(), 2);
        assert!(log2[1].total_words > words1);
        assert!(log2[1].word_delta > 0, "word delta should be positive");

        // Restore to c1 -> file shrinks back.
        let n = vcs.restore(&c1, None).unwrap();
        assert_eq!(n, 1);
        let restored = fs::read_to_string(root.join("ch1.md")).unwrap();
        assert_eq!(restored, "第一章 林惊羽出场");

        // c2 still exists in history.
        assert!(vcs.diff(&c2).is_ok());
    }

    #[test]
    fn diff_reports_added_changed_removed() {
        let root = temp_root("diff");
        fs::write(root.join("a.md"), "aaa bbb").unwrap();
        let mut vcs = FictionVcs::open(&root).unwrap();
        let _c1 = vcs.commit("first").unwrap();

        fs::write(root.join("a.md"), "aaa bbb ccc").unwrap(); // changed +1 word
        fs::write(root.join("b.md"), "new file here").unwrap(); // added 3
        let c2 = vcs.commit("second").unwrap();

        let d = vcs.diff(&c2).unwrap();
        let by_path: BTreeMap<_, _> = d.files.iter().map(|f| (f.path.as_str(), f)).collect();
        assert_eq!(by_path["a.md"].status, "changed");
        assert_eq!(by_path["a.md"].word_delta, 1);
        assert_eq!(by_path["b.md"].status, "added");
        assert_eq!(by_path["b.md"].word_delta, 3);
        assert_eq!(d.total_word_delta, 4);

        // Remove a.md and commit -> removed status.
        fs::remove_file(root.join("a.md")).unwrap();
        let c3 = vcs.commit("third").unwrap();
        let d3 = vcs.diff(&c3).unwrap();
        let removed = d3.files.iter().find(|f| f.path == "a.md").unwrap();
        assert_eq!(removed.status, "removed");
        assert!(removed.word_delta < 0);
    }

    #[test]
    fn chapter_history_tracks_one_file() {
        let root = temp_root("chap");
        fs::write(root.join("ch1.md"), "one two").unwrap();
        let mut vcs = FictionVcs::open(&root).unwrap();
        vcs.commit("v1").unwrap();
        fs::write(root.join("ch1.md"), "one two three four").unwrap();
        vcs.commit("v2").unwrap();

        let hist = vcs.chapter_history("ch1.md");
        assert_eq!(hist.len(), 2);
        assert_eq!(hist[0].words, 2);
        assert_eq!(hist[1].words, 4);
        assert!(hist.iter().all(|p| p.present));

        // A file that never existed.
        let none = vcs.chapter_history("ghost.md");
        assert!(none.iter().all(|p| !p.present));
    }

    #[test]
    fn restore_single_file_only() {
        let root = temp_root("single");
        fs::write(root.join("a.md"), "alpha").unwrap();
        fs::write(root.join("b.md"), "beta").unwrap();
        let mut vcs = FictionVcs::open(&root).unwrap();
        let c1 = vcs.commit("base").unwrap();
        fs::write(root.join("a.md"), "ALPHA changed").unwrap();
        fs::write(root.join("b.md"), "BETA changed").unwrap();

        // Restore only a.md.
        vcs.restore(&c1, Some("a.md")).unwrap();
        assert_eq!(fs::read_to_string(root.join("a.md")).unwrap(), "alpha");
        // b.md untouched.
        assert_eq!(
            fs::read_to_string(root.join("b.md")).unwrap(),
            "BETA changed"
        );
    }

    #[test]
    fn single_file_restore_rejects_parent_traversal() {
        let root = temp_root("restore-traversal");
        let outside = temp_root("restore-outside");
        let victim = outside.join("victim.md");
        fs::write(root.join("inside.md"), "inside").unwrap();
        fs::write(&victim, "keep").unwrap();
        let mut vcs = FictionVcs::open(&root).unwrap();
        let commit = vcs.commit("base").unwrap();
        let attack = format!(
            "../{}/victim.md",
            outside.file_name().unwrap().to_string_lossy()
        );

        let error = vcs.restore(&commit, Some(&attack)).unwrap_err();
        assert!(error.is(na_common::ErrorKind::SandboxViolation), "{error}");
        assert_eq!(fs::read_to_string(victim).unwrap(), "keep");
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

    fn skip_unavailable_symlink(error: &std::io::Error) -> bool {
        error.kind() == std::io::ErrorKind::PermissionDenied
            || error.kind() == std::io::ErrorKind::Unsupported
    }

    #[test]
    fn single_file_restore_rejects_reserved_normalized_aliases() {
        let root = temp_root("restore-reserved");
        fs::write(root.join("chapter.md"), "snapshot").unwrap();
        let mut vcs = FictionVcs::open(&root).unwrap();
        let commit = vcs.commit("base").unwrap();
        let absolute = root.join(".na/state.json").to_string_lossy().into_owned();

        for attack in [
            ".na/state.json",
            "./.NA/state.json",
            "draft/../.Na-VcS/state.json",
            ".GiT/config",
            absolute.as_str(),
        ] {
            let error = vcs.restore(&commit, Some(attack)).unwrap_err();
            assert!(
                error.is(na_common::ErrorKind::SecurityBlocked),
                "{attack}: {error}"
            );
        }

        #[cfg(windows)]
        for attack in [".na./state.json", ".git /config", ".na-vcs./objects"] {
            let error = vcs.restore(&commit, Some(attack)).unwrap_err();
            assert!(
                error.is(na_common::ErrorKind::SecurityBlocked),
                "{attack}: {error}"
            );
        }
    }

    #[test]
    fn single_file_restore_never_follows_destination_symlink() {
        let root = temp_root("restore-direct-link");
        let target = root.join("actual.md");
        let destination = root.join("chapter.md");
        fs::write(&destination, "snapshot").unwrap();
        let mut vcs = FictionVcs::open(&root).unwrap();
        let commit = vcs.commit("base").unwrap();
        fs::remove_file(&destination).unwrap();
        fs::write(&target, "current target").unwrap();
        if let Err(error) = symlink_file(&target, &destination) {
            if skip_unavailable_symlink(&error) {
                return;
            }
            panic!("creating test symlink failed: {error}");
        }

        let error = vcs.restore(&commit, Some("chapter.md")).unwrap_err();
        assert!(error.is(na_common::ErrorKind::SandboxViolation), "{error}");
        assert_eq!(fs::read_to_string(&target).unwrap(), "current target");
        let _ = fs::remove_file(destination);
    }

    #[test]
    fn restore_handles_file_directory_transitions_in_both_directions() {
        let root = temp_root("restore-kind-transition");
        let node = root.join("node");
        fs::write(&node, "file snapshot").unwrap();
        let mut vcs = FictionVcs::open(&root).unwrap();
        let file_commit = vcs.commit("file").unwrap();

        fs::remove_file(&node).unwrap();
        fs::create_dir(&node).unwrap();
        fs::write(node.join("child.md"), "directory snapshot").unwrap();
        let directory_commit = vcs.commit("directory").unwrap();

        vcs.restore(&file_commit, None).unwrap();
        assert!(node.is_file());
        assert_eq!(fs::read_to_string(&node).unwrap(), "file snapshot");

        vcs.restore(&directory_commit, None).unwrap();
        assert!(node.is_dir());
        assert_eq!(
            fs::read_to_string(node.join("child.md")).unwrap(),
            "directory snapshot"
        );
    }

    #[test]
    fn single_path_restore_replaces_non_directory_ancestor_transactionally() {
        let root = temp_root("restore-single-transition");
        fs::create_dir(root.join("node")).unwrap();
        fs::write(root.join("node/child.md"), "snapshot").unwrap();
        let mut vcs = FictionVcs::open(&root).unwrap();
        let commit = vcs.commit("directory").unwrap();
        fs::remove_dir_all(root.join("node")).unwrap();
        fs::write(root.join("node"), "conflicting file").unwrap();

        vcs.restore(&commit, Some("node/child.md")).unwrap();
        assert_eq!(
            fs::read_to_string(root.join("node/child.md")).unwrap(),
            "snapshot"
        );
    }

    #[test]
    fn single_path_restore_removes_directory_when_path_is_absent() {
        let root = temp_root("restore-absent-directory");
        fs::write(root.join("chapter.md"), "snapshot").unwrap();
        let mut vcs = FictionVcs::open(&root).unwrap();
        let commit = vcs.commit("without notes").unwrap();
        fs::create_dir(root.join("notes")).unwrap();
        fs::write(root.join("notes/draft.md"), "untracked").unwrap();

        assert_eq!(vcs.restore(&commit, Some("notes")).unwrap(), 0);
        assert!(!root.join("notes").exists());
    }

    #[test]
    fn full_restore_rejects_parent_symlink_escape() {
        let root = temp_root("restore-link");
        let outside = temp_root("restore-link-outside");
        fs::create_dir_all(root.join("chapter")).unwrap();
        fs::write(root.join("chapter/one.md"), "snapshot").unwrap();
        let mut vcs = FictionVcs::open(&root).unwrap();
        let commit = vcs.commit("base").unwrap();
        fs::remove_dir_all(root.join("chapter")).unwrap();
        let link = root.join("chapter");
        if let Err(error) = symlink_dir(&outside, &link) {
            if skip_unavailable_symlink(&error) {
                return;
            }
            panic!("creating test symlink failed: {error}");
        }

        let error = vcs.restore(&commit, None).unwrap_err();
        assert!(error.is(na_common::ErrorKind::SandboxViolation), "{error}");
        assert!(!outside.join("one.md").exists());
        let _ = fs::remove_file(link);
    }

    #[test]
    fn diff_unknown_commit_is_not_found() {
        let root = temp_root("unknown");
        let vcs = FictionVcs::open(&root).unwrap();
        let err = vcs
            .diff(&CheckpointId::from_existing("ckpt_nope"))
            .unwrap_err();
        assert!(err.is(na_common::ErrorKind::NotFound));
    }

    #[test]
    fn commits_reload_after_reopen() {
        let root = temp_root("reload");
        {
            fs::write(root.join("x.md"), "hello").unwrap();
            let mut vcs = FictionVcs::open(&root).unwrap();
            vcs.commit("persisted").unwrap();
        }
        let vcs2 = FictionVcs::open(&root).unwrap();
        assert_eq!(vcs2.len(), 1);
        assert_eq!(vcs2.log()[0].message, "persisted");
    }

    #[test]
    fn commit_skips_reserved_case_variants_and_reopens() {
        let root = temp_root("reserved-case");
        for directory in [".NA", ".GIT"] {
            fs::create_dir_all(root.join(directory)).unwrap();
            fs::write(root.join(directory).join("state"), "internal").unwrap();
        }
        fs::write(root.join("chapter.md"), "manuscript").unwrap();
        {
            let mut vcs = FictionVcs::open(&root).unwrap();
            let commit = vcs.commit("clean").unwrap();
            let record = vcs
                .commits
                .iter()
                .find(|candidate| candidate.id == commit)
                .unwrap();
            assert_eq!(record.files.len(), 1);
            assert_eq!(record.files[0].path, "chapter.md");
        }
        assert_eq!(FictionVcs::open(&root).unwrap().len(), 1);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn commit_log_rejects_object_path_traversal_and_reserved_targets() {
        for (relative_path, hash) in [
            ("leak.md", "../../../outside-file"),
            (".na/memory.jsonl", "0-cbf29ce484222325"),
        ] {
            let root = temp_root("crafted-log");
            let vcs_dir = root.join(VCS_DIR);
            fs::create_dir_all(vcs_dir.join("objects")).unwrap();
            let commit = Commit {
                id: CheckpointId::new(),
                message: "crafted".into(),
                ts: now_millis(),
                branch: "main".into(),
                files: vec![FileRecord {
                    path: relative_path.into(),
                    hash: hash.into(),
                    words: 0,
                }],
            };
            fs::write(
                vcs_dir.join("commits.jsonl"),
                format!("{}\n", serde_json::to_string(&commit).unwrap()),
            )
            .unwrap();

            let error = FictionVcs::open(&root).unwrap_err();
            assert!(error.is(na_common::ErrorKind::Serialization), "{error}");
            let _ = fs::remove_dir_all(root);
        }
    }

    #[test]
    fn commit_log_rejects_path_ancestor_conflicts() {
        let root = temp_root("crafted-log-ancestor");
        let vcs_dir = root.join(VCS_DIR);
        fs::create_dir_all(vcs_dir.join("objects")).unwrap();
        let hash = content_hash(b"content");
        let commit = Commit {
            id: CheckpointId::new(),
            message: "crafted".into(),
            ts: now_millis(),
            branch: "main".into(),
            files: vec![
                FileRecord {
                    path: "chapter".into(),
                    hash: hash.clone(),
                    words: 0,
                },
                FileRecord {
                    path: "chapter/one.md".into(),
                    hash,
                    words: 0,
                },
            ],
        };
        fs::write(
            vcs_dir.join("commits.jsonl"),
            format!("{}\n", serde_json::to_string(&commit).unwrap()),
        )
        .unwrap();

        let error = FictionVcs::open(&root).unwrap_err();
        assert!(error.is(na_common::ErrorKind::Serialization), "{error}");
    }

    #[test]
    fn torn_final_commit_record_is_ignored_and_repaired_on_next_commit() {
        let root = temp_root("torn-log");
        fs::write(root.join("chapter.md"), "first").unwrap();
        let mut vcs = FictionVcs::open(&root).unwrap();
        vcs.commit("first").unwrap();
        drop(vcs);
        let log_path = root.join(VCS_DIR).join("commits.jsonl");
        let mut bytes = fs::read(&log_path).unwrap();
        bytes.extend_from_slice(br#"{"id":"ckpt_torn""#);
        fs::write(&log_path, bytes).unwrap();

        let mut reopened = FictionVcs::open(&root).unwrap();
        assert_eq!(reopened.len(), 1);
        fs::write(root.join("chapter.md"), "second").unwrap();
        reopened.commit("second").unwrap();
        drop(reopened);

        let repaired = fs::read_to_string(&log_path).unwrap();
        assert!(repaired.ends_with('\n'));
        assert!(!repaired.contains("ckpt_torn"));
        assert_eq!(FictionVcs::open(&root).unwrap().len(), 2);
    }

    #[test]
    fn completed_corrupt_commit_record_is_rejected() {
        let root = temp_root("corrupt-log-line");
        let vcs_dir = root.join(VCS_DIR);
        fs::create_dir_all(vcs_dir.join("objects")).unwrap();
        fs::write(vcs_dir.join("commits.jsonl"), "{not-json}\n").unwrap();
        let error = FictionVcs::open(&root).unwrap_err();
        assert!(error.is(na_common::ErrorKind::Serialization), "{error}");
    }

    #[test]
    fn symlinked_vcs_metadata_is_rejected() {
        for target_name in ["commits.jsonl", "state.json"] {
            let root = temp_root("metadata-link");
            let vcs_dir = root.join(VCS_DIR);
            fs::create_dir_all(vcs_dir.join("objects")).unwrap();
            let outside = root.join("outside-metadata");
            fs::write(&outside, "{}").unwrap();
            let link = vcs_dir.join(target_name);
            if let Err(error) = symlink_file(&outside, &link) {
                if skip_unavailable_symlink(&error) {
                    return;
                }
                panic!("creating test symlink failed: {error}");
            }
            let error = FictionVcs::open(&root).unwrap_err();
            assert!(error.is(na_common::ErrorKind::SandboxViolation), "{error}");
            let _ = fs::remove_file(link);
        }

        let root = temp_root("objects-link");
        let vcs_dir = root.join(VCS_DIR);
        let outside = root.join("outside-objects");
        fs::create_dir_all(&vcs_dir).unwrap();
        fs::create_dir_all(&outside).unwrap();
        let link = vcs_dir.join("objects");
        if let Err(error) = symlink_dir(&outside, &link) {
            if skip_unavailable_symlink(&error) {
                return;
            }
            panic!("creating test symlink failed: {error}");
        }
        let error = FictionVcs::open(&root).unwrap_err();
        assert!(error.is(na_common::ErrorKind::SandboxViolation), "{error}");
        let _ = fs::remove_file(link);
    }

    #[cfg(not(windows))]
    #[test]
    fn unix_platform_valid_names_round_trip() {
        let root = temp_root("unix-names");
        for name in ["colon:name.md", r"backslash\name.md"] {
            fs::write(root.join(name), name).unwrap();
        }
        let mut vcs = FictionVcs::open(&root).unwrap();
        let commit = vcs.commit("platform names").unwrap();
        drop(vcs);
        for name in ["colon:name.md", r"backslash\name.md"] {
            fs::write(root.join(name), "changed").unwrap();
        }
        let vcs = FictionVcs::open(&root).unwrap();
        vcs.restore(&commit, None).unwrap();
        for name in ["colon:name.md", r"backslash\name.md"] {
            assert_eq!(fs::read_to_string(root.join(name)).unwrap(), name);
        }
    }

    #[test]
    fn corrupt_vcs_blob_aborts_restore_before_workspace_mutation() {
        let root = temp_root("corrupt-vcs-object");
        let file = root.join("chapter.md");
        fs::write(&file, "snapshot").unwrap();
        let mut vcs = FictionVcs::open(&root).unwrap();
        let commit = vcs.commit("snapshot").unwrap();
        let hash = vcs.commits[0].files[0].hash.clone();
        fs::write(vcs.objects_dir.join(hash), "corrupt").unwrap();
        fs::write(&file, "current unsaved work").unwrap();

        let error = vcs.restore(&commit, None).unwrap_err();
        assert!(error.is(na_common::ErrorKind::Serialization), "{error}");
        assert_eq!(fs::read_to_string(&file).unwrap(), "current unsaved work");
        let _ = fs::remove_dir_all(root);
    }

    // ---- Tool wrappers ----

    fn ctx(tag: &str) -> ToolContext {
        ToolContextBuilder::new(temp_root(tag)).build().unwrap()
    }

    #[tokio::test]
    async fn commit_and_log_tools() {
        let c = ctx("tools");
        let abs = c.jail.resolve("ch1.md").unwrap();
        fs::write(&abs, "第一章 内容").unwrap();

        let res = GitCommitTool
            .execute(json!({ "message": "初稿" }), &c)
            .await
            .unwrap();
        assert!(res.ok);
        assert!(res.data["total_words"].as_u64().unwrap() > 0);

        let log = GitLogTool.execute(json!({}), &c).await.unwrap();
        assert!(log.ok);
        assert_eq!(log.data["count"], 1);
        assert!(log.content.contains("初稿"));
    }

    #[tokio::test]
    async fn restore_tool_round_trip() {
        let c = ctx("restore_tool");
        let abs = c.jail.resolve("ch1.md").unwrap();
        fs::write(&abs, "original").unwrap();
        let commit = GitCommitTool
            .execute(json!({ "message": "v1" }), &c)
            .await
            .unwrap();
        let id = commit.data["id"].as_str().unwrap().to_string();

        fs::write(&abs, "changed").unwrap();
        let res = GitRestoreTool
            .execute(json!({ "id": id, "path": "ch1.md" }), &c)
            .await
            .unwrap();
        assert!(res.ok);
        assert_eq!(fs::read_to_string(&abs).unwrap(), "original");
    }

    #[tokio::test]
    async fn diff_tool_shows_delta() {
        let c = ctx("diff_tool");
        let abs = c.jail.resolve("a.md").unwrap();
        fs::write(&abs, "one two").unwrap();
        GitCommitTool
            .execute(json!({ "message": "c1" }), &c)
            .await
            .unwrap();
        fs::write(&abs, "one two three four five").unwrap();
        let c2 = GitCommitTool
            .execute(json!({ "message": "c2" }), &c)
            .await
            .unwrap();
        let id = c2.data["id"].as_str().unwrap().to_string();

        let res = GitDiffTool.execute(json!({ "id": id }), &c).await.unwrap();
        assert!(res.ok);
        assert!(res.content.contains("net:"));
        assert_eq!(res.data["total_word_delta"], 3);
    }

    // ---- branches / variants ----

    #[test]
    fn default_branch_is_main() {
        let root = temp_root("defbranch");
        let vcs = FictionVcs::open(&root).unwrap();
        assert_eq!(vcs.current_branch(), "main");
        assert_eq!(vcs.branches(), vec!["main".to_string()]);
    }

    #[test]
    fn branch_switch_and_independent_commits() {
        let root = temp_root("branchcommit");
        fs::write(root.join("ch1.md"), "base line").unwrap();
        let mut vcs = FictionVcs::open(&root).unwrap();
        let main_c1 = vcs.commit("main v1").unwrap();
        assert_eq!(vcs.commits[0].branch, "main");

        // Fork an alternate plot line.
        vcs.branch("alt-ending").unwrap();
        assert_eq!(vcs.current_branch(), "alt-ending");
        fs::write(root.join("ch1.md"), "base line plus alternate twist").unwrap();
        let alt_c = vcs.commit("alt twist").unwrap();

        // The alt branch log shows only its own commit.
        let alt_log = vcs.log();
        assert_eq!(alt_log.len(), 1);
        assert_eq!(alt_log[0].id, alt_c);

        // Back on main, commit independently.
        vcs.switch("main").unwrap();
        assert_eq!(vcs.current_branch(), "main");
        fs::write(root.join("ch1.md"), "base line main continues").unwrap();
        let main_c2 = vcs.commit("main v2").unwrap();

        let main_log = vcs.log();
        assert_eq!(main_log.len(), 2);
        assert_eq!(main_log[0].id, main_c1);
        assert_eq!(main_log[1].id, main_c2);

        // Branch list contains both, sorted, deduped.
        assert_eq!(
            vcs.branches(),
            vec!["alt-ending".to_string(), "main".to_string()]
        );

        // diff() of main v2 uses the same-branch predecessor (main v1), NOT the
        // intervening alt commit.
        let d = vcs.diff(&main_c2).unwrap();
        assert_eq!(d.parent.as_ref(), Some(&main_c1));
    }

    #[test]
    fn branch_state_persists_across_reopen() {
        let root = temp_root("branchpersist");
        {
            let mut vcs = FictionVcs::open(&root).unwrap();
            vcs.branch("draft2").unwrap();
        }
        let vcs2 = FictionVcs::open(&root).unwrap();
        assert_eq!(vcs2.current_branch(), "draft2");
        assert!(vcs2.branches().contains(&"draft2".to_string()));
    }

    #[test]
    fn branch_errors_on_duplicate_and_empty() {
        let root = temp_root("brancherr");
        let mut vcs = FictionVcs::open(&root).unwrap();
        assert!(vcs
            .branch("")
            .unwrap_err()
            .is(na_common::ErrorKind::InvalidInput));
        vcs.branch("x").unwrap();
        assert!(vcs
            .branch("x")
            .unwrap_err()
            .is(na_common::ErrorKind::Conflict));
        // main already exists too.
        assert!(vcs
            .branch("main")
            .unwrap_err()
            .is(na_common::ErrorKind::Conflict));
    }

    #[test]
    fn failed_state_writes_do_not_change_live_branch_or_tags() {
        let branch_root = temp_root("branch-write-failure");
        let mut branch_vcs = FictionVcs::open(&branch_root).unwrap();
        fs::create_dir(&branch_vcs.state_path).unwrap();
        assert!(branch_vcs.branch("uncommitted").is_err());
        assert_eq!(branch_vcs.current_branch(), "main");
        assert_eq!(branch_vcs.branches(), vec!["main".to_string()]);

        let switch_root = temp_root("switch-write-failure");
        let mut switch_vcs = FictionVcs::open(&switch_root).unwrap();
        switch_vcs.branch("draft").unwrap();
        fs::remove_file(&switch_vcs.state_path).unwrap();
        fs::create_dir(&switch_vcs.state_path).unwrap();
        assert!(switch_vcs.switch("main").is_err());
        assert_eq!(switch_vcs.current_branch(), "draft");

        let tag_root = temp_root("tag-write-failure");
        fs::write(tag_root.join("chapter.md"), "text").unwrap();
        let mut tag_vcs = FictionVcs::open(&tag_root).unwrap();
        let commit = tag_vcs.commit("commit").unwrap();
        fs::create_dir(&tag_vcs.state_path).unwrap();
        assert!(tag_vcs.tag(&commit, "uncommitted-tag").is_err());
        assert!(tag_vcs.tags().is_empty());

        let _ = fs::remove_dir_all(branch_root);
        let _ = fs::remove_dir_all(switch_root);
        let _ = fs::remove_dir_all(tag_root);
    }

    #[test]
    fn switch_to_unknown_branch_errors() {
        let root = temp_root("switcherr");
        let mut vcs = FictionVcs::open(&root).unwrap();
        assert!(vcs
            .switch("ghost")
            .unwrap_err()
            .is(na_common::ErrorKind::NotFound));
    }

    // ---- line-level diff ----

    #[test]
    fn diff_lines_added_removed_context() {
        let root = temp_root("linediff");
        fs::write(root.join("ch.md"), "alpha\nbeta\ngamma\n").unwrap();
        let mut vcs = FictionVcs::open(&root).unwrap();
        let a = vcs.commit("a").unwrap();
        // Remove "beta", change nothing else, add "delta" at the end.
        fs::write(root.join("ch.md"), "alpha\ngamma\ndelta\n").unwrap();
        let b = vcs.commit("b").unwrap();

        let d = vcs.diff_lines(&a, &b, "ch.md").unwrap();
        assert_eq!(d.path, "ch.md");
        assert_eq!(d.removed, 1);
        assert_eq!(d.added, 1);

        let removed: Vec<&str> = d
            .lines
            .iter()
            .filter(|l| l.kind == "removed")
            .map(|l| l.text.as_str())
            .collect();
        assert_eq!(removed, vec!["beta"]);
        let added: Vec<&str> = d
            .lines
            .iter()
            .filter(|l| l.kind == "added")
            .map(|l| l.text.as_str())
            .collect();
        assert_eq!(added, vec!["delta"]);
        // "alpha" and "gamma" survive as context.
        let context: Vec<&str> = d
            .lines
            .iter()
            .filter(|l| l.kind == "context")
            .map(|l| l.text.as_str())
            .collect();
        assert_eq!(context, vec!["alpha", "gamma"]);
    }

    #[test]
    fn diff_lines_handles_added_file() {
        let root = temp_root("linediff_add");
        fs::write(root.join("keep.md"), "x").unwrap();
        let mut vcs = FictionVcs::open(&root).unwrap();
        let a = vcs.commit("a").unwrap();
        fs::write(root.join("new.md"), "line1\nline2").unwrap();
        let b = vcs.commit("b").unwrap();

        // new.md did not exist at `a` -> all lines added.
        let d = vcs.diff_lines(&a, &b, "new.md").unwrap();
        assert_eq!(d.added, 2);
        assert_eq!(d.removed, 0);
    }

    #[test]
    fn diff_lines_cjk_content() {
        let root = temp_root("linediff_cjk");
        fs::write(root.join("c.md"), "第一行\n第二行\n").unwrap();
        let mut vcs = FictionVcs::open(&root).unwrap();
        let a = vcs.commit("a").unwrap();
        fs::write(root.join("c.md"), "第一行\n第二行改动\n").unwrap();
        let b = vcs.commit("b").unwrap();
        let d = vcs.diff_lines(&a, &b, "c.md").unwrap();
        assert_eq!(d.added, 1);
        assert_eq!(d.removed, 1);
        assert!(d
            .lines
            .iter()
            .any(|l| l.kind == "added" && l.text == "第二行改动"));
        assert!(d
            .lines
            .iter()
            .any(|l| l.kind == "context" && l.text == "第一行"));
    }

    #[test]
    fn diff_lines_unknown_revision_errors() {
        let root = temp_root("linediff_err");
        fs::write(root.join("a.md"), "x").unwrap();
        let mut vcs = FictionVcs::open(&root).unwrap();
        let a = vcs.commit("a").unwrap();
        let err = vcs
            .diff_lines(&a, &CheckpointId::from_existing("ckpt_nope"), "a.md")
            .unwrap_err();
        assert!(err.is(na_common::ErrorKind::NotFound));
    }

    // ---- tags ----

    #[test]
    fn tag_and_retag() {
        let root = temp_root("tags");
        fs::write(root.join("a.md"), "one").unwrap();
        let mut vcs = FictionVcs::open(&root).unwrap();
        let c1 = vcs.commit("c1").unwrap();
        fs::write(root.join("a.md"), "one two").unwrap();
        let c2 = vcs.commit("c2").unwrap();

        vcs.tag(&c1, "卷一终").unwrap();
        assert_eq!(vcs.tags().len(), 1);
        assert_eq!(vcs.resolve_tag("卷一终"), Some(c1.clone()));

        // Re-tagging the same label moves it.
        vcs.tag(&c2, "卷一终").unwrap();
        assert_eq!(vcs.tags().len(), 1);
        assert_eq!(vcs.resolve_tag("卷一终"), Some(c2));

        // Empty label and unknown commit are rejected.
        assert!(vcs
            .tag(&c1, "  ")
            .unwrap_err()
            .is(na_common::ErrorKind::InvalidInput));
        assert!(vcs
            .tag(&CheckpointId::from_existing("ckpt_x"), "t")
            .unwrap_err()
            .is(na_common::ErrorKind::NotFound));
    }

    #[test]
    fn tags_persist_across_reopen() {
        let root = temp_root("tagspersist");
        let c = {
            fs::write(root.join("a.md"), "x").unwrap();
            let mut vcs = FictionVcs::open(&root).unwrap();
            let c = vcs.commit("c").unwrap();
            vcs.tag(&c, "milestone").unwrap();
            c
        };
        let vcs2 = FictionVcs::open(&root).unwrap();
        assert_eq!(vcs2.resolve_tag("milestone"), Some(c));
    }

    // ---- GitBranchTool ----

    #[tokio::test]
    async fn branch_tool_create_switch_list() {
        let c = ctx("branchtool");
        let abs = c.jail.resolve("ch1.md").unwrap();
        fs::write(&abs, "base").unwrap();
        GitCommitTool
            .execute(json!({ "message": "v1" }), &c)
            .await
            .unwrap();

        // create
        let created = GitBranchTool
            .execute(json!({ "action": "create", "name": "alt" }), &c)
            .await
            .unwrap();
        assert!(created.ok, "{}", created.content);
        assert_eq!(created.data["current"], "alt");

        // list shows both with the current marked.
        let listed = GitBranchTool
            .execute(json!({ "action": "list" }), &c)
            .await
            .unwrap();
        assert!(listed.ok);
        assert!(listed.content.contains("* alt"));
        assert!(listed.content.contains("main"));

        // switch back to main.
        let switched = GitBranchTool
            .execute(json!({ "action": "switch", "name": "main" }), &c)
            .await
            .unwrap();
        assert!(switched.ok);
        assert_eq!(switched.data["current"], "main");
    }

    #[tokio::test]
    async fn branch_tool_create_requires_name() {
        let c = ctx("branchtool_noname");
        let err = GitBranchTool
            .execute(json!({ "action": "create" }), &c)
            .await
            .unwrap_err();
        assert!(err.is(na_common::ErrorKind::InvalidInput));
    }
}
