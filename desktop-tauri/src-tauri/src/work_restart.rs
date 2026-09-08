//! Start a new manuscript with durable reference material from an existing work.
//! The source is read-only; old runtime state is kept as text, never loaded into
//! the new story cursor. All writes occur before WorkStore publishes the work.

use std::fs;
use std::io::{ErrorKind, Write};
use std::path::Path;

use na_common::{CoreError, Result};
use na_library::{KnowledgeStore, WorkMeta};
use na_memory::{MemoryKind, MemoryStore};
use na_runtime::StoryStateManager;
use serde::Serialize;

const STORY_REFERENCE: &str = "restart-reference/story_state.txt";
const REFERENCE_FILES: &[&str] = &[
    ".na/memory.jsonl",
    ".na/style_profiles.json",
    ".na/active_style.json",
    ".na/writer.md",
    ".na/outline.md",
    "writer.md",
    "outline.md",
    "world.md",
    "characters.md",
    "foreshadows.md",
    "世界观.md",
    "角色设定.md",
    "人物设定.md",
    "故事大纲.md",
    "大纲.md",
    "伏笔.md",
];

#[derive(Debug, Serialize)]
pub(crate) struct RestartReport {
    pub memory_count: usize,
    pub knowledge_base_count: usize,
    pub story_reference_saved: bool,
}

pub(crate) fn prepare_restart(source: &WorkMeta, target: &WorkMeta) -> Result<RestartReport> {
    // Inspect each component so directory junctions cannot cross work boundaries.
    check_path(&source.workspace_dir)?;
    for name in REFERENCE_FILES {
        copy_optional(
            &source.workspace_dir.join(name),
            &target.workspace_dir.join(name),
        )?;
    }
    for name in ["planning", "notes", "设定", "策划", "restart-reference"] {
        copy_optional(
            &source.workspace_dir.join(name),
            &target.workspace_dir.join(name),
        )?;
    }
    copy_optional(&source.knowledge_dir, &target.knowledge_dir)?;

    // Validate the copied durable stores before activating them. Never skip
    // damaged memory records and claim that all memories were transferred.
    let memory_path = target.workspace_dir.join(".na/memory.jsonl");
    let mut memory = MemoryStore::open(&memory_path)
        .map_err(|error| error.with_context("原作品记忆无法完整复制，原作品已保留"))?;
    // JSONL readers accept a final record without a newline; future appends
    // still need a separator. Normalize only the new copy, after validation.
    if memory_path.exists() {
        let bytes = fs::read(&memory_path)?;
        if !bytes.is_empty() && !bytes.ends_with(b"\n") {
            fs::OpenOptions::new()
                .append(true)
                .open(&memory_path)?
                .write_all(b"\n")?;
        }
    }
    let memory_count = memory.all().len();
    let knowledge = KnowledgeStore::open(&target.knowledge_dir)?;
    let bases = knowledge.list_bases()?;
    for base in &bases {
        knowledge.open_base(&base.id)?;
    }

    let story_reference_saved = copy_optional(
        &source.workspace_dir.join("story_state.json"),
        &target.workspace_dir.join(STORY_REFERENCE),
    )?;
    if story_reference_saved {
        let text = fs::read_to_string(target.workspace_dir.join(STORY_REFERENCE))?;
        memory.save(
            MemoryKind::Other,
            "重开参考：原作品剧情设定",
            "原作品的角色、世界观和剧情记录，仅供重写参考；新书从第一章开始。",
            text,
            vec!["重开参考".into()],
            5,
        )?;
    }

    // Generate a fresh, valid state even when the original is malformed JSON.
    let mut story = StoryStateManager::open(target.workspace_dir.join("story_state.json"))?;
    story.state.meta.title = target.title.clone();
    story.state.meta.genre = target.genre.clone();
    story.save()?;
    fs::create_dir_all(target.workspace_dir.join("book"))?;

    // Honor legacy .na/writer.md instead of letting the default mask it.
    let writer = target.workspace_dir.join("writer.md");
    if !writer.exists() {
        copy_optional(&target.workspace_dir.join(".na/writer.md"), &writer)?;
    }
    super::ensure_default_writer_md(&target.workspace_dir)?;
    let mut instructions = fs::OpenOptions::new().append(true).open(writer)?;
    instructions.write_all("\n\n## 保留记忆重新开书\n\n这是一部从第一章重新创作的新作品。使用 memory_recall / memory_get 查阅继承的设定与大纲；旧记忆中的事件和完成记录仅作重写参考，章节进度以本作品的 book/ 和剧情状态为准。\n".as_bytes())?;
    if story_reference_saved {
        instructions.write_all(format!(
            "原作品剧情资料已完整保存在 {STORY_REFERENCE}，写作前请读取其中的人物与世界观设定，按照作者本次要求重新安排事件。该文件仅作文本参考，不要直接覆盖新的 story_state.json。\n"
        ).as_bytes())?;
    }
    instructions.sync_all()?;
    Ok(RestartReport {
        memory_count,
        knowledge_base_count: bases.len(),
        story_reference_saved,
    })
}

fn check_path(path: &Path) -> Result<()> {
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) => {
                let linked = metadata.file_type().is_symlink();
                #[cfg(windows)]
                let linked = {
                    use std::os::windows::fs::MetadataExt;
                    linked || metadata.file_attributes() & 0x400 != 0 // reparse point
                };
                if linked {
                    return Err(CoreError::invalid_input(format!(
                        "无法复制链接路径，请先将资料保存为作品内的普通文件：{}",
                        ancestor.display()
                    )));
                }
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => {
                return Err(CoreError::from(error).with_context(path.display().to_string()))
            }
        }
    }
    Ok(())
}

fn copy_optional(source: &Path, target: &Path) -> Result<bool> {
    check_path(source)?;
    let metadata = match fs::symlink_metadata(source) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(CoreError::from(error).with_context(source.display().to_string())),
    };
    if metadata.is_dir() {
        fs::create_dir_all(target)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            // A committed deletion's residual files are not live knowledge.
            if entry.file_name().to_string_lossy().starts_with(".deleted-") {
                continue;
            }
            copy_optional(&entry.path(), &target.join(entry.file_name()))?;
        }
    } else if metadata.is_file() {
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(source, target).map_err(|error| {
            CoreError::from(error).with_context(format!("复制 {}", source.display()))
        })?;
    } else {
        return Err(CoreError::invalid_input(format!(
            "无法复制特殊文件：{}",
            source.display()
        )));
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use na_library::{KnowledgeKind, WorkStore};

    #[test]
    fn restart_preserves_recall_and_references_without_reusing_broken_state() {
        for original_state in [
            r#"{"meta":{"last_chapter":9},"timeline":{"events":["玄机子合身合道"]},"foreshadows":["旧格式伏笔"]}"#,
            r#"{"characters": "截断但保留的人物资料""#,
        ] {
            let root = tempfile::tempdir().unwrap();
            let mut works = WorkStore::open(root.path()).unwrap();
            let source = works.create("原作", "简介", "修真", "原著").unwrap();
            let source_memory = source.workspace_dir.join(".na/memory.jsonl");
            let mut memory = MemoryStore::open(&source_memory).unwrap();
            let id = memory
                .save(
                    MemoryKind::Character,
                    "师尊",
                    "身合天道",
                    "师尊暗中护持主角",
                    vec!["师尊".into()],
                    5,
                )
                .unwrap();
            let archived_id = memory
                .save(
                    MemoryKind::Outline,
                    "旧大纲",
                    "旧版本",
                    "废弃设定",
                    vec![],
                    1,
                )
                .unwrap();
            memory.archive(&archived_id, true).unwrap();
            let original_entries = memory.all().to_vec();
            let mut no_final_newline = fs::read(&source_memory).unwrap();
            assert_eq!(no_final_newline.pop(), Some(b'\n'));
            fs::write(&source_memory, no_final_newline).unwrap();
            let original_memory_bytes = fs::read(&source_memory).unwrap();
            fs::write(
                source.workspace_dir.join("story_state.json"),
                original_state,
            )
            .unwrap();
            // The first chapter has already been deleted. A later chapter and
            // corrupt checkpoint data must never be inherited into the new work.
            fs::create_dir(source.workspace_dir.join("book")).unwrap();
            fs::write(source.workspace_dir.join("book/ch2.md"), "旧稿").unwrap();
            fs::write(source.workspace_dir.join(".na/checkpoints.json"), "broken").unwrap();
            fs::write(
                source.sessions_dir.join("old-session.json"),
                "old conversation",
            )
            .unwrap();
            fs::write(source.workspace_dir.join(".na/writer.md"), "作者的文风规范").unwrap();
            fs::write(source.workspace_dir.join("outline.md"), "第一章：合道").unwrap();
            fs::write(source.workspace_dir.join(".na/style_profiles.json"), "[]").unwrap();
            let knowledge = KnowledgeStore::open(&source.knowledge_dir).unwrap();
            let kb = knowledge.create_base("设定", "原作资料").unwrap();
            let mut base = knowledge.open_base(&kb.id).unwrap();
            base.add(
                KnowledgeKind::Worldbuilding,
                "天道",
                "天道不是物质",
                "user",
                vec![],
            )
            .unwrap();
            let original_kb =
                fs::read(source.knowledge_dir.join(&kb.id).join("entries.jsonl")).unwrap();
            let other = works.create("另一本", "", "", "").unwrap();
            assert_eq!(works.active_id(), Some(other.id.as_str()));
            let (new, report) = works
                .create_from(&source.id, "重开", |old, new| {
                    let report = prepare_restart(old, new)?;
                    super::super::build_active_context(new).map_err(CoreError::invalid_input)?;
                    Ok(report)
                })
                .unwrap();
            assert_eq!(report.memory_count, 2);
            assert_eq!(report.knowledge_base_count, 1);
            assert!(report.story_reference_saved);
            let copied = MemoryStore::open(new.workspace_dir.join(".na/memory.jsonl")).unwrap();
            assert_eq!(&copied.all()[..2], original_entries.as_slice());
            assert_eq!(copied.get(&id), memory.get(&id));
            assert!(!copied.recall("师尊", 5, None, false).is_empty());
            assert!(copied.get(&archived_id).unwrap().archived);
            assert_eq!(
                fs::read(new.workspace_dir.join(STORY_REFERENCE)).unwrap(),
                original_state.as_bytes()
            );
            assert_eq!(
                fs::read(source.workspace_dir.join("story_state.json")).unwrap(),
                original_state.as_bytes()
            );
            assert_eq!(fs::read(&source_memory).unwrap(), original_memory_bytes);
            assert!(source.workspace_dir.join("book/ch2.md").is_file());
            assert_eq!(
                fs::read_dir(new.workspace_dir.join("book"))
                    .unwrap()
                    .count(),
                0
            );
            assert_eq!(fs::read_dir(&new.sessions_dir).unwrap().count(), 0);
            let story =
                StoryStateManager::open(new.workspace_dir.join("story_state.json")).unwrap();
            assert_eq!(story.state.meta.title, "重开");
            assert_eq!(story.state.meta.last_chapter, 0);
            assert_eq!(story.state.timeline.current_chapter, 0);
            assert!(story.state.timeline.events.is_empty());
            let profile = na_runtime::ProjectProfile::load(&new.workspace_dir);
            let writer = profile.writer_md.unwrap();
            assert!(writer.contains("作者的文风规范"));
            assert!(writer.contains(STORY_REFERENCE));
            assert_eq!(profile.outline_md.as_deref(), Some("第一章：合道"));
            let new_knowledge = KnowledgeStore::open(&new.knowledge_dir).unwrap();
            assert!(!new_knowledge.search_active("天道", 5).unwrap().is_empty());
            new_knowledge.delete_base(&kb.id).unwrap();
            assert_eq!(
                fs::read(source.knowledge_dir.join(&kb.id).join("entries.jsonl")).unwrap(),
                original_kb
            );
            assert_eq!(works.get(&source.id), Some(&source));
            assert_eq!(
                WorkStore::open(root.path()).unwrap().active_id(),
                Some(new.id.as_str())
            );
        }
    }

    #[test]
    fn restart_without_state_or_memories_creates_a_valid_empty_work() {
        let root = tempfile::tempdir().unwrap();
        let mut works = WorkStore::open(root.path()).unwrap();
        let source = works.create("空白原作", "", "", "").unwrap();
        let (new, report) = works
            .create_from(&source.id, "重开", prepare_restart)
            .unwrap();
        assert_eq!(report.memory_count, 0);
        assert!(!report.story_reference_saved);
        assert!(!source.workspace_dir.join(".na").exists());
        super::super::build_active_context(&new).unwrap();
        StoryStateManager::open(new.workspace_dir.join("story_state.json")).unwrap();
    }

    #[test]
    fn damaged_memory_aborts_restart_without_modifying_source_or_selection() {
        let root = tempfile::tempdir().unwrap();
        let mut works = WorkStore::open(root.path()).unwrap();
        let source = works.create("原作", "", "", "").unwrap();
        fs::create_dir(source.workspace_dir.join(".na")).unwrap();
        fs::write(source.workspace_dir.join(".na/memory.jsonl"), "not json").unwrap();
        let before = works.list();
        let result = works.create_from(&source.id, "重开", prepare_restart);
        assert!(result.is_err());
        assert_eq!(works.list(), before);
        assert_eq!(fs::read_dir(root.path().join("works")).unwrap().count(), 1);
        assert_eq!(
            fs::read_to_string(source.workspace_dir.join(".na/memory.jsonl")).unwrap(),
            "not json"
        );
    }

    #[test]
    fn restart_rejects_linked_reference_directories() {
        let root = tempfile::tempdir().unwrap();
        let mut works = WorkStore::open(root.path()).unwrap();
        let source = works.create("原作", "", "", "").unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("private.txt"), "outside").unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(outside.path(), source.workspace_dir.join("notes"))
            .unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(outside.path(), source.workspace_dir.join("notes")).unwrap();
        assert!(works
            .create_from(&source.id, "重开", prepare_restart)
            .is_err());
        assert_eq!(works.list().len(), 1);
        assert!(outside.path().join("private.txt").exists());
    }
}
