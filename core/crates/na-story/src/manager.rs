//! Story state manager for loading, saving, and querying story state.

use crate::state::*;
use na_common::{CoreError, Result};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Story state manager with atomic file operations.
pub struct StoryStateManager {
    pub state: StoryState,
    state_path: PathBuf,
}

impl StoryStateManager {
    /// Open or create story state at the given path.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let state_path = path.as_ref().to_path_buf();
        if let Some(parent) = state_path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }
        let state = if state_path.exists() {
            let content = fs::read_to_string(&state_path)?;
            serde_json::from_str(&content)?
        } else {
            StoryState::default()
        };

        Ok(StoryStateManager { state, state_path })
    }

    /// Save state to disk with a flushed atomic replacement.
    pub fn save(&self) -> Result<()> {
        let content = serde_json::to_string_pretty(&self.state)?;
        let parent = self
            .state_path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|error| {
            CoreError::from(error).with_context("creating temporary story state")
        })?;
        temporary.write_all(content.as_bytes()).map_err(|error| {
            CoreError::from(error).with_context("writing temporary story state")
        })?;
        temporary.as_file().sync_all().map_err(|error| {
            CoreError::from(error).with_context("flushing temporary story state")
        })?;
        temporary
            .persist(&self.state_path)
            .map_err(|error| CoreError::from(error.error).with_context("replacing story state"))?;
        Ok(())
    }

    /// Prepare context package for a given chapter.
    pub fn prepare_context(&self, chapter_num: u32) -> ContextPackage {
        ContextPackage {
            chapter_num,
            relevant_characters: self.state.characters.values().cloned().collect(),
            recent_events: self
                .state
                .timeline
                .events
                .iter()
                .rev()
                .take(3)
                .cloned()
                .collect(),
            hard_constraints: self.active_constraints(Severity::High),
            pending_foreshadows: self.pending_foreshadows(),
            chapter_goal: self.state.current_chapter_goal.clone(),
        }
    }

    /// Get active constraints with severity >= min_severity, sorted by severity descending.
    pub fn active_constraints(&self, min_severity: Severity) -> Vec<Constraint> {
        let mut constraints: Vec<_> = self
            .state
            .hard_constraints
            .iter()
            .filter(|c| c.severity >= min_severity)
            .cloned()
            .collect();
        constraints.sort_by(|a, b| b.severity.cmp(&a.severity));
        constraints
    }

    /// Get pending foreshadows (Planted or Hinted).
    pub fn pending_foreshadows(&self) -> Vec<ForeshadowTracker> {
        self.state
            .foreshadows
            .iter()
            .filter(|f| {
                matches!(
                    f.status,
                    ForeshadowStatus::Planted | ForeshadowStatus::Hinted
                )
            })
            .cloned()
            .collect()
    }
}

/// Context package for chapter generation.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ContextPackage {
    pub chapter_num: u32,
    pub relevant_characters: Vec<CharacterState>,
    pub recent_events: Vec<TimelineEvent>,
    pub hard_constraints: Vec<Constraint>,
    pub pending_foreshadows: Vec<ForeshadowTracker>,
    pub chapter_goal: Option<ChapterGoal>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    fn temp_path(tag: &str) -> PathBuf {
        let mut p = env::temp_dir();
        p.push(format!("na_story_test_{}_{}", tag, na_common::next_id("t")));
        p
    }

    #[test]
    fn open_and_save() {
        let path = temp_path("open_save");
        let mut mgr = StoryStateManager::open(&path).unwrap();
        mgr.state.meta.title = "Test Story".to_string();
        mgr.save().unwrap();

        let mgr2 = StoryStateManager::open(&path).unwrap();
        assert_eq!(mgr2.state.meta.title, "Test Story");

        fs::remove_file(&path).ok();
    }

    #[test]
    fn repeated_save_replaces_the_previous_state() {
        let path = temp_path("repeat").join("nested/story_state.json");
        let mut manager = StoryStateManager::open(&path).unwrap();
        manager.state.meta.title = "First".to_string();
        manager.save().unwrap();
        manager.state.meta.title = "Second".to_string();
        manager.state.meta.last_chapter = 2;
        manager.save().unwrap();

        let reopened = StoryStateManager::open(&path).unwrap();
        assert_eq!(reopened.state.meta.title, "Second");
        assert_eq!(reopened.state.meta.last_chapter, 2);
        fs::remove_dir_all(path.parent().unwrap().parent().unwrap()).ok();
    }

    #[test]
    fn legacy_foreshadows_allow_successive_chapters_after_first_save() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("story_state.json");
        let description = "玄机子留给顾长生的生锈铜钥匙及洞府试炼";
        let mut manager = StoryStateManager::open(&path).unwrap();
        manager.state.meta.title = "这个修仙界真的正常吗".to_string();
        manager.advance_chapter();
        manager.save().unwrap();

        // Model-authored state after chapter one contains plain text entries.
        let mut json: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        json["foreshadows"] = serde_json::json!([description]);
        json["hard_constraints"] = serde_json::json!(["主角不能凭空知道秘密"]);
        json["soft_preferences"] = serde_json::json!(["对话简洁"]);
        let original = serde_json::to_vec_pretty(&json).unwrap();
        fs::write(&path, &original).unwrap();

        let migrated = StoryStateManager::open(&path).unwrap();
        assert_eq!(fs::read(&path).unwrap(), original);
        let foreshadow_id = migrated.state.foreshadows[0].id.clone();
        for chapter in 2..=4 {
            let mut manager = StoryStateManager::open(&path).unwrap();
            assert_eq!(manager.state.meta.last_chapter + 1, chapter);
            manager.set_chapter_goal(chapter, format!("续写第{chapter}章"));
            manager.save().unwrap();
            let context = manager.prepare_context(chapter);
            let prompt = crate::render_state_sync_prompt(&context);
            assert!(prompt.contains(description));
            assert!(prompt.contains("埋设章节未知"));
            assert!(!prompt.contains("埋于第0章"));
            assert!(prompt.contains("主角不能凭空知道秘密"));
            assert_eq!(context.pending_foreshadows[0].id, foreshadow_id);
            assert_eq!(context.chapter_goal.unwrap().chapter, chapter);
            manager.add_timeline_event(chapter, format!("完成第{chapter}章"));
            manager.advance_chapter();
            manager.save().unwrap();
        }
        let final_state = StoryStateManager::open(&path).unwrap().state;
        assert_eq!(final_state.meta.last_chapter, 4);
        assert_eq!(final_state.timeline.events.len(), 3);
        assert_eq!(final_state.foreshadows[0].description, description);
        assert_eq!(final_state.soft_preferences[0].description, "对话简洁");
    }

    #[test]
    fn constraint_priority_ordering() {
        let path = temp_path("constraints");
        let mut mgr = StoryStateManager::open(&path).unwrap();

        mgr.state.hard_constraints.push(Constraint {
            id: "c1".to_string(),
            description: "Medium".to_string(),
            severity: Severity::Medium,
        });
        mgr.state.hard_constraints.push(Constraint {
            id: "c2".to_string(),
            description: "Critical".to_string(),
            severity: Severity::Critical,
        });
        mgr.state.hard_constraints.push(Constraint {
            id: "c3".to_string(),
            description: "High".to_string(),
            severity: Severity::High,
        });

        let active = mgr.active_constraints(Severity::Medium);
        assert_eq!(active.len(), 3);
        assert_eq!(active[0].severity, Severity::Critical);
        assert_eq!(active[1].severity, Severity::High);
        assert_eq!(active[2].severity, Severity::Medium);

        fs::remove_file(&path).ok();
    }

    #[test]
    fn pending_foreshadows_filter() {
        let path = temp_path("foreshadows");
        let mut mgr = StoryStateManager::open(&path).unwrap();

        mgr.state.foreshadows.push(ForeshadowTracker {
            id: "f1".to_string(),
            description: "Planted".to_string(),
            planted_at: 1,
            status: ForeshadowStatus::Planted,
        });
        mgr.state.foreshadows.push(ForeshadowTracker {
            id: "f2".to_string(),
            description: "Resolved".to_string(),
            planted_at: 1,
            status: ForeshadowStatus::Resolved,
        });
        mgr.state.foreshadows.push(ForeshadowTracker {
            id: "f3".to_string(),
            description: "Hinted".to_string(),
            planted_at: 2,
            status: ForeshadowStatus::Hinted,
        });

        let pending = mgr.pending_foreshadows();
        assert_eq!(pending.len(), 2);
        assert!(pending.iter().any(|f| f.id == "f1"));
        assert!(pending.iter().any(|f| f.id == "f3"));

        fs::remove_file(&path).ok();
    }
}
