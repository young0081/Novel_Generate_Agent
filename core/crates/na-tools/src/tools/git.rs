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

use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Component, Path, PathBuf};

use na_common::time::now_millis;
use na_common::{json, CheckpointId, CoreError, Json, Result};
use na_memory::content_hash;
use na_sandbox::Capability;
use serde::{Deserialize, Serialize};

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
    workspace_root: PathBuf,
    vcs_dir: PathBuf,
    objects_dir: PathBuf,
    commits_path: PathBuf,
    state_path: PathBuf,
    commits: Vec<Commit>,
    state: VcsState,
}

impl FictionVcs {
    /// Open (or initialize) the store under `<workspace_root>/.na-vcs`.
    pub fn open(workspace_root: impl AsRef<Path>) -> Result<Self> {
        let workspace_root = workspace_root.as_ref().to_path_buf();
        let vcs_dir = workspace_root.join(VCS_DIR);
        let objects_dir = vcs_dir.join("objects");
        let commits_path = vcs_dir.join("commits.jsonl");
        let state_path = vcs_dir.join("state.json");

        fs::create_dir_all(&objects_dir)
            .map_err(|e| CoreError::from(e).with_context("creating .na-vcs/objects"))?;

        let commits = load_commits(&commits_path)?;
        let mut state = load_state(&state_path)?;
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
            Some(rel) => {
                let rel = normalize_rel(rel);
                match commit.files.iter().find(|f| f.path == rel) {
                    Some(rec) => {
                        self.write_file_from_blob(&rec.path, &rec.hash)?;
                        Ok(1)
                    }
                    None => {
                        // File absent in the target commit -> delete it locally.
                        let abs = self.workspace_root.join(rel_to_pathbuf(&rel));
                        if abs.exists() {
                            fs::remove_file(&abs).map_err(|e| {
                                CoreError::from(e)
                                    .with_context(format!("deleting {rel} during restore"))
                            })?;
                        }
                        Ok(0)
                    }
                }
            }
            None => {
                // Whole-workspace restore.
                let desired: std::collections::BTreeSet<&str> =
                    commit.files.iter().map(|f| f.path.as_str()).collect();
                // Delete extras.
                let mut existing = Vec::new();
                self.collect_rel_files(&self.workspace_root, &mut existing)?;
                for rel in &existing {
                    if !desired.contains(rel.as_str()) {
                        let abs = self.workspace_root.join(rel_to_pathbuf(rel));
                        if abs.exists() {
                            let _ = fs::remove_file(&abs);
                        }
                    }
                }
                // Write all.
                for rec in &commit.files {
                    self.write_file_from_blob(&rec.path, &rec.hash)?;
                }
                Ok(commit.files.len())
            }
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
        self.state.branches.push(name.to_string());
        self.state.current_branch = name.to_string();
        self.save_state()
    }

    /// Switch the working branch to an existing `name`. Errors if it is unknown.
    pub fn switch(&mut self, name: &str) -> Result<()> {
        let name = name.trim();
        if !self.state.branches.iter().any(|b| b == name) {
            return Err(CoreError::not_found(format!("no such branch {name:?}")));
        }
        self.state.current_branch = name.to_string();
        self.save_state()
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
        if let Some(existing) = self.state.tags.iter_mut().find(|t| t.label == label) {
            existing.commit = rev.clone();
        } else {
            self.state.tags.push(Tag {
                label: label.to_string(),
                commit: rev.clone(),
            });
        }
        self.save_state()
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

    fn write_file_from_blob(&self, rel: &str, hash: &str) -> Result<()> {
        let blob = self.read_blob(hash)?;
        let abs = self.workspace_root.join(rel_to_pathbuf(rel));
        if let Some(parent) = abs.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| CoreError::from(e).with_context(format!("creating dirs for {rel}")))?;
        }
        fs::write(&abs, &blob).map_err(|e| {
            CoreError::from(e).with_context(format!("writing {rel} during restore"))
        })?;
        Ok(())
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

    fn collect_rel_files(&self, dir: &Path, out: &mut Vec<String>) -> Result<()> {
        if !dir.exists() {
            return Ok(());
        }
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
                self.collect_rel_files(&path, out)?;
            } else if ft.is_file() {
                out.push(self.rel_path(&path)?);
            }
        }
        Ok(())
    }

    /// Ignore the VCS dir, the `.na` state dir, and `.git`.
    fn is_ignored(&self, path: &Path) -> bool {
        if path.starts_with(&self.vcs_dir) {
            return true;
        }
        matches!(
            path.file_name().and_then(|n| n.to_str()),
            Some(VCS_DIR) | Some(".na") | Some(".git")
        )
    }

    fn rel_path(&self, path: &Path) -> Result<String> {
        let rel = path.strip_prefix(&self.workspace_root).map_err(|_| {
            CoreError::internal(format!("{} not under workspace root", path.display()))
        })?;
        let mut parts = Vec::new();
        for comp in rel.components() {
            match comp {
                Component::Normal(os) => parts.push(os.to_string_lossy().into_owned()),
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
        fs::read(self.blob_path(hash))
            .map_err(|e| CoreError::from(e).with_context(format!("reading blob {hash}")))
    }

    fn append_commit(&self, commit: &Commit) -> Result<()> {
        let line = serde_json::to_string(commit)?;
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.commits_path)
            .map_err(|e| CoreError::from(e).with_context("opening commits.jsonl"))?;
        file.write_all(line.as_bytes())
            .and_then(|_| file.write_all(b"\n"))
            .map_err(|e| CoreError::from(e).with_context("appending commit"))?;
        Ok(())
    }

    /// Persist the branch/tag state atomically (write-then-rename).
    fn save_state(&self) -> Result<()> {
        let json = serde_json::to_string_pretty(&self.state)?;
        let tmp = self.state_path.with_extension("json.tmp");
        fs::write(&tmp, json.as_bytes())
            .map_err(|e| CoreError::from(e).with_context("writing .na-vcs/state.json"))?;
        fs::rename(&tmp, &self.state_path)
            .map_err(|e| CoreError::from(e).with_context("committing .na-vcs/state.json"))?;
        Ok(())
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
    let mut s = rel.replace('\\', "/");
    while let Some(stripped) = s.strip_prefix("./") {
        s = stripped.to_string();
    }
    s.trim_start_matches('/').to_string()
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

/// Load commits from the JSON-Lines log (missing file => empty), sorted by time.
fn load_commits(path: &Path) -> Result<Vec<Commit>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file = fs::File::open(path)
        .map_err(|e| CoreError::from(e).with_context("opening commits.jsonl"))?;
    let reader = BufReader::new(file);
    let mut out = Vec::new();
    for (lineno, line) in reader.lines().enumerate() {
        let line = line.map_err(|e| CoreError::from(e).with_context("reading commits.jsonl"))?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let commit: Commit = serde_json::from_str(trimmed).map_err(|e| {
            CoreError::from(e).with_context(format!("parsing commit at line {}", lineno + 1))
        })?;
        out.push(commit);
    }
    out.sort_by_key(|c| c.ts);
    Ok(out)
}

/// Load the branch/tag state (missing file => defaults).
fn load_state(path: &Path) -> Result<VcsState> {
    if !path.exists() {
        return Ok(VcsState::default());
    }
    let text = fs::read_to_string(path)
        .map_err(|e| CoreError::from(e).with_context("reading .na-vcs/state.json"))?;
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
