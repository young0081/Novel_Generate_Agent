//! Path jail: confine every filesystem access to a canonical root directory.
//!
//! The agent's tools (read/write/edit/list files) must never be able to touch
//! anything outside the workspace the user opened. [`PathJail`] is the single
//! choke point that enforces this: every requested path is joined to the root
//! and **lexically normalized** (`.` and `..` resolved purely by string math,
//! without touching the disk), then checked for escapes. Any attempt to climb
//! above the root — via `..`, an absolute path, a Windows drive prefix, or a
//! UNC/`\\?\` prefix — is rejected with a [`CoreError::sandbox`].
//!
//! Normalization is intentionally lexical (not [`std::fs::canonicalize`]) for
//! two reasons:
//!   1. The target file may not exist yet (a write that creates it), and
//!      `canonicalize` requires existence.
//!   2. We must not be fooled — or blocked — by symlinks at resolve time; the
//!      decision should depend only on the textual path the model asked for.
//!
//! The root itself *is* canonicalized once (and created if missing) so that
//! [`contains`](PathJail::contains) / [`relative`](PathJail::relative) work
//! against a stable absolute anchor.

use std::path::{Component, Path, PathBuf};

use na_common::{CoreError, Result};

/// A directory boundary that all resolved paths are guaranteed to stay within.
#[derive(Debug, Clone)]
pub struct PathJail {
    /// Canonical, absolute root. All resolved paths live at or below this.
    root: PathBuf,
}

impl PathJail {
    /// Create a jail rooted at `root`.
    ///
    /// The directory is created (recursively) if it does not exist, then
    /// canonicalized to an absolute path with symlinks resolved. Failing to
    /// create or canonicalize the root is reported as a [`CoreError`].
    pub fn new(root: impl AsRef<Path>) -> Result<PathJail> {
        let root = root.as_ref();

        // Ensure the root exists so it can be canonicalized and later used.
        if !root.exists() {
            std::fs::create_dir_all(root).map_err(|e| {
                CoreError::from(e).with_context(format!("creating sandbox root {}", root.display()))
            })?;
        }

        let canonical = std::fs::canonicalize(root).map_err(|e| {
            CoreError::from(e)
                .with_context(format!("canonicalizing sandbox root {}", root.display()))
        })?;

        if !canonical.is_dir() {
            return Err(CoreError::invalid_input(format!(
                "sandbox root is not a directory: {}",
                canonical.display()
            )));
        }

        Ok(PathJail { root: canonical })
    }

    /// The canonical absolute root of this jail.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Resolve a model/tool-supplied path into an absolute path guaranteed to
    /// live within the jail.
    ///
    /// The `requested` string is interpreted relative to the root, except that
    /// a verbatim absolute path is allowed *only* if it already points inside
    /// the root (otherwise it is rejected as an escape). `.` and `..` segments
    /// are resolved lexically; any `..` that would climb above the root causes
    /// a [`CoreError::sandbox`].
    ///
    /// The returned path is **not** required to exist (so callers may create
    /// the file). It is, however, always normalized and within the root.
    pub fn resolve(&self, requested: &str) -> Result<PathBuf> {
        let req_path = Path::new(requested);

        // Decide on the "base" the request is resolved against, and the slice
        // of components we still need to fold in.
        //
        // * Relative paths        -> base = root, fold all components.
        // * Absolute / drive / UNC -> only accept if it textually lands inside
        //                             root; we normalize the *whole* absolute
        //                             path and then verify containment.
        let normalized = if is_absolute_like(req_path) {
            // Normalize the absolute path on its own, then require it to be
            // within the (already canonical) root.
            let abs = lexically_normalize_absolute(req_path).ok_or_else(|| {
                CoreError::sandbox(format!("absolute path escapes sandbox root: {requested}"))
            })?;
            if !path_within(&self.root, &abs) {
                return Err(CoreError::sandbox(format!(
                    "absolute path is outside sandbox root: {requested}"
                )));
            }
            abs
        } else {
            // Relative: fold the request's components onto the root, refusing
            // to climb above it.
            fold_relative(&self.root, req_path).ok_or_else(|| {
                CoreError::sandbox(format!("path escapes sandbox root: {requested}"))
            })?
        };

        // Final defensive check: the normalized result must be within root.
        if !path_within(&self.root, &normalized) {
            return Err(CoreError::sandbox(format!(
                "resolved path escapes sandbox root: {requested}"
            )));
        }

        Ok(normalized)
    }

    /// Whether `p` (treated lexically) is the root or a descendant of it.
    ///
    /// `p` is normalized first; relative paths are interpreted against the
    /// root, matching [`resolve`](PathJail::resolve)'s view of the world.
    pub fn contains(&self, p: &Path) -> bool {
        let normalized = if is_absolute_like(p) {
            match lexically_normalize_absolute(p) {
                Some(abs) => abs,
                None => return false,
            }
        } else {
            match fold_relative(&self.root, p) {
                Some(abs) => abs,
                None => return false,
            }
        };
        path_within(&self.root, &normalized)
    }

    /// If `p` is within the jail, return its path relative to the root using
    /// forward slashes (`/`); otherwise `None`. The root itself maps to `""`.
    pub fn relative(&self, p: &Path) -> Option<String> {
        let normalized = if is_absolute_like(p) {
            lexically_normalize_absolute(p)?
        } else {
            fold_relative(&self.root, p)?
        };

        let rel = normalized.strip_prefix(&self.root).ok()?;
        let mut parts: Vec<String> = Vec::new();
        for comp in rel.components() {
            // After normalization there should be no `.`/`..`; ignore any stray
            // prefix/root components defensively and keep only real segments.
            if let Component::Normal(s) = comp {
                parts.push(s.to_string_lossy().into_owned());
            }
        }
        Some(parts.join("/"))
    }
}

/// Is this path "absolute-like": a real absolute path, or one carrying a
/// Windows prefix (drive `C:`, UNC `\\server\share`, `\\?\...`)?
///
/// On every platform we also treat a path *starting with a separator* as
/// absolute-like, and a path that begins with a `X:` drive specifier as
/// absolute-like, so that a Windows-style escape attempt is caught even when
/// the tests run on a Unix host (where `Path::is_absolute` would say `false`).
fn is_absolute_like(p: &Path) -> bool {
    if p.is_absolute() {
        return true;
    }
    // Inspect the leading components for a prefix or root we should treat as
    // absolute regardless of host platform.
    let mut comps = p.components();
    match comps.next() {
        Some(Component::Prefix(_)) | Some(Component::RootDir) => return true,
        _ => {}
    }

    // Cross-platform textual heuristics (so a Unix test host still rejects
    // Windows-style inputs):
    let s = p.as_os_str().to_string_lossy();
    let bytes = s.as_bytes();
    // Leading slash or backslash -> absolute root.
    if matches!(bytes.first(), Some(b'/') | Some(b'\\')) {
        return true;
    }
    // Drive letter like "C:" or "C:\" or "C:/".
    if bytes.len() >= 2 && bytes[1] == b':' {
        let c = bytes[0];
        if c.is_ascii_alphabetic() {
            return true;
        }
    }
    false
}

/// Lexically normalize an *absolute-like* path into an absolute [`PathBuf`],
/// resolving `.`/`..` by string math. Returns `None` if `..` would climb above
/// the path's own anchor (prefix/root) — that is itself treated as an escape.
fn lexically_normalize_absolute(p: &Path) -> Option<PathBuf> {
    let mut anchor = PathBuf::new();
    let mut stack: Vec<std::ffi::OsString> = Vec::new();
    let mut anchored = false;

    for comp in p.components() {
        match comp {
            Component::Prefix(prefix) => {
                anchor.push(prefix.as_os_str());
            }
            Component::RootDir => {
                anchor.push(Component::RootDir.as_os_str());
                anchored = true;
            }
            Component::CurDir => {}
            Component::ParentDir => {
                // Climbing above the anchor is an escape: `pop` yields `None`
                // when the stack is empty, propagating the rejection.
                stack.pop()?;
            }
            Component::Normal(s) => stack.push(s.to_os_string()),
        }
    }

    // A drive prefix without an explicit root (e.g. bare "C:") still anchors
    // an absolute-like path for our purposes.
    let mut out = anchor;
    if !anchored && out.as_os_str().is_empty() {
        // Path had neither prefix nor root but reached here; treat as failure
        // (caller only invokes this for absolute-like paths).
        return None;
    }
    for part in stack {
        out.push(part);
    }
    Some(out)
}

/// Fold a *relative* request onto `base`, resolving `.`/`..` lexically and
/// refusing to climb above `base`. `base` must already be absolute & normal.
fn fold_relative(base: &Path, requested: &Path) -> Option<PathBuf> {
    // Start the stack from the base's own components so `..` can only consume
    // segments the request itself added — never any part of the root.
    let base_len = base.components().count();
    let mut comps: Vec<Component> = base.components().collect();

    for comp in requested.components() {
        match comp {
            // A relative path should not contain these, but be defensive: any
            // prefix/root inside a "relative" request is an escape attempt.
            Component::Prefix(_) | Component::RootDir => return None,
            Component::CurDir => {}
            Component::ParentDir => {
                if comps.len() <= base_len {
                    // Would pop into (or above) the root: escape.
                    return None;
                }
                comps.pop();
            }
            Component::Normal(_) => comps.push(comp),
        }
    }

    let mut out = PathBuf::new();
    for c in comps {
        out.push(c.as_os_str());
    }
    Some(out)
}

/// Is `candidate` equal to `root` or a descendant of it (lexical prefix on
/// component boundaries)?
fn path_within(root: &Path, candidate: &Path) -> bool {
    if candidate == root {
        return true;
    }
    candidate.starts_with(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a jail in a unique temp directory.
    fn temp_jail() -> PathJail {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "na_sandbox_jail_{}_{}",
            std::process::id(),
            na_common::next_id("t")
        ));
        let jail = PathJail::new(&dir).expect("create jail");
        // Sanity: root canonicalized & exists.
        assert!(jail.root().is_absolute());
        jail
    }

    #[test]
    fn root_is_created_and_canonical() {
        let jail = temp_jail();
        assert!(jail.root().exists());
        assert!(jail.root().is_dir());
    }

    #[test]
    fn accepts_normal_in_root_paths() {
        let jail = temp_jail();
        let p = jail.resolve("chapter1.md").expect("simple file");
        assert!(p.starts_with(jail.root()));
        assert!(p.ends_with("chapter1.md"));

        let nested = jail.resolve("book/part1/ch1.md").expect("nested");
        assert!(nested.starts_with(jail.root()));
        assert!(jail.contains(&nested));

        // `.` and harmless `..` that stays inside are fine.
        let inside = jail.resolve("a/b/../c.txt").expect("stays inside");
        assert!(inside.starts_with(jail.root()));
        assert!(inside.ends_with("a/c.txt"));

        let cur = jail.resolve("./notes.txt").expect("curdir");
        assert!(cur.ends_with("notes.txt"));
    }

    #[test]
    fn empty_resolves_to_root() {
        let jail = temp_jail();
        let p = jail.resolve("").expect("empty -> root");
        assert_eq!(p, jail.root());
        let dot = jail.resolve(".").expect("dot -> root");
        assert_eq!(dot, jail.root());
    }

    #[test]
    fn rejects_dotdot_escape() {
        let jail = temp_jail();
        let err = jail.resolve("../../etc/passwd").unwrap_err();
        assert!(err.is(na_common::ErrorKind::SandboxViolation), "{err}");
    }

    #[test]
    fn rejects_interleaved_escape() {
        let jail = temp_jail();
        // a/../../b climbs one level above root after cancelling `a`.
        let err = jail.resolve("a/../../b").unwrap_err();
        assert!(err.is(na_common::ErrorKind::SandboxViolation), "{err}");
    }

    #[test]
    fn rejects_single_dotdot_at_root() {
        let jail = temp_jail();
        assert!(jail.resolve("..").is_err());
        assert!(jail.resolve("../").is_err());
    }

    #[test]
    fn rejects_leading_slash_absolute() {
        let jail = temp_jail();
        // A leading-slash absolute path that does not point into root.
        let err = jail.resolve("/etc/passwd").unwrap_err();
        assert!(err.is(na_common::ErrorKind::SandboxViolation), "{err}");
    }

    #[test]
    fn rejects_windows_drive_path() {
        let jail = temp_jail();
        let err = jail.resolve("C:\\Windows\\System32").unwrap_err();
        assert!(err.is(na_common::ErrorKind::SandboxViolation), "{err}");
        // Forward-slash drive form too.
        assert!(jail.resolve("C:/Windows").is_err());
    }

    #[test]
    fn rejects_backslash_dotdot_escape() {
        let jail = temp_jail();
        // "..\.." style escape. Backslash is only a separator on Windows, but
        // our absolute/relative detection plus component folding must still
        // refuse to climb above the root on any host.
        // On Unix this is a single weird filename; ensure it never escapes.
        let res = jail.resolve("..\\..");
        match res {
            Ok(p) => {
                // If treated as a literal name, it must still be inside root.
                assert!(p.starts_with(jail.root()), "{}", p.display());
            }
            Err(e) => assert!(e.is(na_common::ErrorKind::SandboxViolation)),
        }

        // The unambiguous forward-slash equivalent must always be rejected.
        assert!(jail.resolve("../..").is_err());
    }

    #[test]
    fn rejects_deep_dotdot_runs() {
        let jail = temp_jail();
        assert!(jail.resolve("x/../../../../../../root").is_err());
        assert!(jail.resolve("../../../").is_err());
    }

    #[test]
    fn absolute_path_inside_root_is_accepted() {
        let jail = temp_jail();
        // Construct an absolute path that genuinely lives inside the root.
        let mut inside = jail.root().to_path_buf();
        inside.push("sub");
        inside.push("file.txt");
        let s = inside.to_string_lossy().to_string();
        let resolved = jail.resolve(&s).expect("absolute inside root ok");
        assert!(resolved.starts_with(jail.root()));
        assert!(resolved.ends_with("sub/file.txt") || resolved.ends_with("sub\\file.txt"));
    }

    #[test]
    fn absolute_with_dotdot_back_into_root_is_accepted() {
        let jail = temp_jail();
        let mut tricky = jail.root().to_path_buf();
        tricky.push("a");
        tricky.push("..");
        tricky.push("b.txt");
        let s = tricky.to_string_lossy().to_string();
        let resolved = jail.resolve(&s).expect("normalizes back into root");
        assert!(resolved.starts_with(jail.root()));
        assert!(resolved.ends_with("b.txt"));
    }

    #[test]
    fn contains_and_relative() {
        let jail = temp_jail();
        let inside = jail.resolve("docs/readme.md").unwrap();
        assert!(jail.contains(&inside));
        assert_eq!(jail.relative(&inside).as_deref(), Some("docs/readme.md"));

        // Root maps to "".
        assert_eq!(jail.relative(jail.root()).as_deref(), Some(""));

        // Something clearly outside.
        let outside = Path::new("/totally/elsewhere");
        assert!(!jail.contains(outside));
        assert_eq!(jail.relative(outside), None);

        // A relative request resolves & is contained.
        assert!(jail.contains(Path::new("a/b/c")));
    }

    #[test]
    fn relative_uses_forward_slashes() {
        let jail = temp_jail();
        let p = jail.resolve("a/b/c.txt").unwrap();
        let rel = jail.relative(&p).unwrap();
        assert_eq!(rel, "a/b/c.txt");
        assert!(!rel.contains('\\'));
    }

    #[test]
    fn new_rejects_file_as_root() {
        // Create a file and try to use it as a root.
        let mut f = std::env::temp_dir();
        f.push(format!("na_sandbox_not_a_dir_{}", na_common::next_id("f")));
        std::fs::write(&f, b"x").expect("write temp file");
        let res = PathJail::new(&f);
        assert!(res.is_err());
        let _ = std::fs::remove_file(&f);
    }
}
