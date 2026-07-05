//! Story state manager for loading, saving, and querying story state.

use crate::state::*;
use na_common::Result;
use std::fs;
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
        let state = if state_path.exists() {
            let content = fs::read_to_string(&state_path)?;
            serde_json::from_str(&content)?
        } else {
            StoryState::default()
        };

        Ok(StoryStateManager { state, state_path })
    }

    /// Save state to disk with atomic write (temp file + rename).
    pub fn save(&self) -> Result<()> {
        let tmp = self.state_path.with_extension("json.tmp");
        let content = serde_json::to_string_pretty(&self.state)?;
        fs::write(&tmp, content)?;
        fs::rename(&tmp, &self.state_path)?;
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
            .filter(|f| matches!(f.status, ForeshadowStatus::Planted | ForeshadowStatus::Hinted))
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
