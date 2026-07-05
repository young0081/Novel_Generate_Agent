//! Project profile — the author's standing instructions that steer every run.
//!
//! A long-form writing project usually has two persistent steering documents:
//!
//! * a **style guide** (`writer.md`) — the author's voice, tone, POV rules,
//!   formatting conventions, things to avoid; and
//! * an **outline** (`outline.md`) — the plan / structure the prose should
//!   follow.
//!
//! [`ProjectProfile::load`] reads whichever of these exist (checking the project
//! root and a `.na/` subfolder), and [`system_messages`](ProjectProfile::system_messages)
//! turns them into [`System`](Message::system) messages so the agent loop injects
//! the author's voice and plan into context at the start of every run — without
//! the user re-pasting them each time.

use std::path::Path;

use crate::message::Message;

/// The loaded steering documents for a project.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProjectProfile {
    /// Contents of the style guide (`writer.md`), if present.
    pub writer_md: Option<String>,
    /// Contents of the outline (`outline.md`), if present.
    pub outline_md: Option<String>,
}

/// Read a file to a trimmed, non-empty string, returning `None` when the file is
/// missing, unreadable, or blank. (A blank steering file should not inject an
/// empty system message.)
fn read_optional(path: &Path) -> Option<String> {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        Err(_) => None,
    }
}

/// Find a steering document by `name`, preferring the project root and falling
/// back to the hidden `.na/` directory.
fn find_doc(jail_root: &Path, name: &str) -> Option<String> {
    read_optional(&jail_root.join(name))
        .or_else(|| read_optional(&jail_root.join(".na").join(name)))
}

impl ProjectProfile {
    /// Construct a profile from explicit contents (mostly for tests).
    pub fn new(writer_md: Option<String>, outline_md: Option<String>) -> Self {
        ProjectProfile {
            writer_md,
            outline_md,
        }
    }

    /// Load the profile from `jail_root`, reading `writer.md` and `outline.md`
    /// (each looked up at the root, then under `.na/`). Missing files are simply
    /// absent from the result; this never errors.
    pub fn load(jail_root: impl AsRef<Path>) -> ProjectProfile {
        let root = jail_root.as_ref();
        ProjectProfile {
            writer_md: find_doc(root, "writer.md"),
            outline_md: find_doc(root, "outline.md"),
        }
    }

    /// Whether the profile carries no steering content.
    pub fn is_empty(&self) -> bool {
        self.writer_md.is_none() && self.outline_md.is_none()
    }

    /// Produce the [`System`](Message::system) messages that inject the loaded
    /// documents into context. The style guide comes first (it governs *how* to
    /// write), then the outline (*what* to write). An empty profile yields no
    /// messages.
    pub fn system_messages(&self) -> Vec<Message> {
        let mut out = Vec::new();
        if let Some(writer) = &self.writer_md {
            out.push(Message::system(format!(
                "# 作者风格指南 (writer.md)\n严格遵循以下写作风格与约定：\n\n{writer}"
            )));
        }
        if let Some(outline) = &self.outline_md {
            out.push(Message::system(format!(
                "# 大纲 (outline.md)\n按以下大纲推进剧情：\n\n{outline}"
            )));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "na_runtime_profile_{}_{}",
            tag,
            na_common::next_id("t")
        ));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn empty_profile_when_no_files() {
        let dir = temp_dir("empty");
        let profile = ProjectProfile::load(&dir);
        assert!(profile.is_empty());
        assert!(profile.system_messages().is_empty());
    }

    #[test]
    fn loads_writer_md_from_root_and_injects_system_message() {
        let dir = temp_dir("writer");
        std::fs::write(
            dir.join("writer.md"),
            "用第三人称限制视角。句子要短。避免陈词滥调。",
        )
        .unwrap();

        let profile = ProjectProfile::load(&dir);
        assert!(!profile.is_empty());
        assert_eq!(
            profile.writer_md.as_deref(),
            Some("用第三人称限制视角。句子要短。避免陈词滥调。")
        );
        assert!(profile.outline_md.is_none());

        let msgs = profile.system_messages();
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].is_system());
        assert!(msgs[0].content.contains("作者风格指南"));
        assert!(msgs[0].content.contains("第三人称限制视角"));
    }

    #[test]
    fn loads_outline_and_writer_in_order() {
        let dir = temp_dir("both");
        std::fs::write(dir.join("writer.md"), "voice rules").unwrap();
        std::fs::write(dir.join("outline.md"), "act 1, act 2, act 3").unwrap();

        let profile = ProjectProfile::load(&dir);
        let msgs = profile.system_messages();
        assert_eq!(msgs.len(), 2);
        // Style guide first, outline second.
        assert!(msgs[0].content.contains("风格指南"));
        assert!(msgs[0].content.contains("voice rules"));
        assert!(msgs[1].content.contains("大纲"));
        assert!(msgs[1].content.contains("act 2"));
    }

    #[test]
    fn falls_back_to_na_subdir() {
        let dir = temp_dir("nadir");
        let na = dir.join(".na");
        std::fs::create_dir_all(&na).unwrap();
        std::fs::write(na.join("writer.md"), "from .na dir").unwrap();

        let profile = ProjectProfile::load(&dir);
        assert_eq!(profile.writer_md.as_deref(), Some("from .na dir"));
    }

    #[test]
    fn root_takes_precedence_over_na_subdir() {
        let dir = temp_dir("precedence");
        let na = dir.join(".na");
        std::fs::create_dir_all(&na).unwrap();
        std::fs::write(dir.join("writer.md"), "root wins").unwrap();
        std::fs::write(na.join("writer.md"), "na loses").unwrap();

        let profile = ProjectProfile::load(&dir);
        assert_eq!(profile.writer_md.as_deref(), Some("root wins"));
    }

    #[test]
    fn blank_file_is_treated_as_absent() {
        let dir = temp_dir("blank");
        std::fs::write(dir.join("writer.md"), "   \n\t\n ").unwrap();
        let profile = ProjectProfile::load(&dir);
        assert!(profile.is_empty());
    }

    #[test]
    fn new_constructs_directly() {
        let p = ProjectProfile::new(Some("w".into()), None);
        assert!(!p.is_empty());
        assert_eq!(p.system_messages().len(), 1);
    }
}
