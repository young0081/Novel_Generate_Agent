//! Project profile — the author's standing instructions that steer every run.
//!
//! A long-form writing project usually has two persistent steering documents:
//!
//! * a **style guide** (`writer.md`) — the author's voice, tone, POV rules,
//!   formatting conventions, things to avoid; and
//! * an **outline** (`outline.md`) — the plan / structure the prose should
//!   follow; and
//! * an active structured [`StyleProfile`] (`.na/active_style.json`) — rules
//!   distilled from reference prose and applied consistently across the book.
//!
//! [`ProjectProfile::load`] reads whichever of these exist (checking the project
//! root and a `.na/` subfolder), and [`system_messages`](ProjectProfile::system_messages)
//! turns them into [`System`](Message::system) messages so the agent loop injects
//! the author's voice and plan into context at the start of every run — without
//! the user re-pasting them each time.

use std::path::Path;

use crate::message::Message;
use crate::style::StyleProfile;

/// The loaded steering documents for a project.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProjectProfile {
    /// Contents of the style guide (`writer.md`), if present.
    pub writer_md: Option<String>,
    /// Contents of the outline (`outline.md`), if present.
    pub outline_md: Option<String>,
    /// The active structured style profile for this work, if selected.
    pub style_profile: Option<StyleProfile>,
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
            style_profile: None,
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
            style_profile: find_active_style(root),
        }
    }

    /// Whether the profile carries no steering content.
    pub fn is_empty(&self) -> bool {
        self.writer_md.is_none() && self.outline_md.is_none() && self.style_profile.is_none()
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
        if let Some(style) = &self.style_profile {
            out.push(Message::system(style.system_prompt()));
        }
        if let Some(outline) = &self.outline_md {
            out.push(Message::system(format!(
                "# 大纲 (outline.md)\n按以下大纲推进剧情：\n\n{outline}"
            )));
        }
        out
    }

    /// Return the Markdown section that explicitly mentions a chapter number.
    /// This keeps a long outline's current beats prominent in the model
    /// context. Headings such as `## 第12章 雨夜`, `## 第十二章 雨夜`, and
    /// `## Chapter 12` work.
    pub fn chapter_outline(&self, chapter: u32) -> Option<String> {
        let outline = self.outline_md.as_deref()?;
        let lines: Vec<&str> = outline.lines().collect();
        let mut start = None;
        let mut heading_level = 0usize;
        for (index, line) in lines.iter().enumerate() {
            let trimmed = line.trim_start();
            let level = trimmed.chars().take_while(|c| *c == '#').count();
            if level == 0 || !trimmed[level..].starts_with(' ') {
                continue;
            }
            if heading_contains_chapter(&trimmed[level..], chapter) {
                start = Some(index);
                heading_level = level;
                break;
            }
        }
        let start = start?;
        let end = lines
            .iter()
            .enumerate()
            .skip(start + 1)
            .find(|(_, line)| {
                let trimmed = line.trim_start();
                let level = trimmed.chars().take_while(|c| *c == '#').count();
                level > 0 && level <= heading_level && trimmed[level..].starts_with(' ')
            })
            .map(|(index, _)| index)
            .unwrap_or(lines.len());
        let section = lines[start..end].join("\n").trim().to_string();
        (!section.is_empty()).then_some(section)
    }
}

fn find_active_style(jail_root: &Path) -> Option<StyleProfile> {
    let raw = read_optional(&jail_root.join(".na").join("active_style.json"))?;
    serde_json::from_str(&raw)
        .ok()
        .filter(|profile: &StyleProfile| {
            !profile.id.trim().is_empty() && !profile.name.trim().is_empty()
        })
}

fn heading_contains_chapter(heading: &str, chapter: u32) -> bool {
    let mut number = 0u32;
    let mut in_number = false;
    for ch in heading.chars() {
        if ch.is_ascii_digit() {
            number = number
                .saturating_mul(10)
                .saturating_add(ch as u32 - '0' as u32);
            in_number = true;
        } else if in_number {
            if number == chapter {
                return true;
            }
            number = 0;
            in_number = false;
        }
    }
    if in_number && number == chapter {
        return true;
    }

    // Chinese outlines commonly spell chapter numbers as "第一章" or
    // "第十二章". Restrict this fallback to an explicit chapter marker so a
    // year or other unrelated numeral in a heading cannot select the section.
    let chars: Vec<char> = heading.chars().collect();
    for (index, ch) in chars.iter().enumerate() {
        if *ch != '第' {
            continue;
        }
        let Some((value, consumed)) = parse_chinese_number(&chars[index + 1..]) else {
            continue;
        };
        let suffix = chars.get(index + 1 + consumed).copied();
        let valid_suffix = matches!(suffix, Some('章' | '节' | '回') | None)
            || suffix.is_some_and(|ch| ch.is_whitespace() || ".、:：-".contains(ch));
        if valid_suffix && value == chapter {
            return true;
        }
    }
    false
}

fn parse_chinese_number(input: &[char]) -> Option<(u32, usize)> {
    fn digit(ch: char) -> Option<u32> {
        Some(match ch {
            '零' | '〇' => 0,
            '一' => 1,
            '二' | '两' => 2,
            '三' => 3,
            '四' => 4,
            '五' => 5,
            '六' => 6,
            '七' => 7,
            '八' => 8,
            '九' => 9,
            _ => return None,
        })
    }

    fn unit(ch: char) -> Option<u32> {
        Some(match ch {
            '十' => 10,
            '百' => 100,
            '千' => 1_000,
            '万' => 10_000,
            '亿' => 100_000_000,
            _ => return None,
        })
    }

    let mut total = 0u32;
    let mut section = 0u32;
    let mut number = 0u32;
    let mut consumed = 0usize;
    let mut saw_number = false;
    for &ch in input {
        if let Some(value) = digit(ch) {
            number = number.saturating_mul(10).saturating_add(value);
            saw_number = true;
            consumed += 1;
            continue;
        }
        let Some(scale) = unit(ch) else {
            break;
        };
        saw_number = true;
        consumed += 1;
        if scale < 10_000 {
            let value = if number == 0 { 1 } else { number };
            section = section.saturating_add(value.saturating_mul(scale));
        } else {
            section = section.saturating_add(number);
            total = total.saturating_add(section.saturating_mul(scale));
            section = 0;
        }
        number = 0;
    }
    if !saw_number {
        return None;
    }
    Some((
        total.saturating_add(section).saturating_add(number),
        consumed,
    ))
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

    #[test]
    fn load_active_style_profile_and_inject_prompt() {
        let dir = temp_dir("active-style");
        std::fs::create_dir_all(dir.join(".na")).unwrap();
        std::fs::write(
            dir.join(".na").join("active_style.json"),
            serde_json::json!({
                "id": "ink",
                "name": "冷峻白描",
                "tone": "克制",
                "humanizer_rules": ["删掉空泛升华"]
            })
            .to_string(),
        )
        .unwrap();
        let profile = ProjectProfile::load(&dir);
        assert_eq!(profile.style_profile.as_ref().unwrap().name, "冷峻白描");
        assert!(profile
            .system_messages()
            .iter()
            .any(|message| message.content.contains("删掉空泛升华")));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn extracts_only_the_requested_chapter_section() {
        let profile = ProjectProfile::new(
            None,
            Some("# 总纲\n## 第1章 开端\n建立冲突\n## 第12章 终局\n决战".into()),
        );
        let chapter = profile.chapter_outline(12).unwrap();
        assert!(chapter.contains("第12章"));
        assert!(chapter.contains("决战"));
        assert!(!chapter.contains("建立冲突"));
        assert!(profile.chapter_outline(2).is_none());
    }

    #[test]
    fn extracts_chinese_numeral_chapter_section() {
        let profile = ProjectProfile::new(
            None,
            Some("## 第一章 开端\n建立冲突\n## 第十二章 终局\n决战".into()),
        );
        let chapter = profile.chapter_outline(12).unwrap();
        assert!(chapter.contains("第十二章"));
        assert!(chapter.contains("决战"));
        assert!(!chapter.contains("建立冲突"));
    }
}
