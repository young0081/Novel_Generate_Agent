//! Helper functions for updating story state during creation workflow.

use crate::state::*;
use crate::manager::StoryStateManager;

impl StoryStateManager {
    /// Add a character to the story state.
    pub fn add_character(&mut self, character: CharacterState) {
        self.state.characters.insert(character.id.clone(), character);
    }

    /// Update character status.
    pub fn update_character_status(&mut self, char_id: &str, new_status: String) {
        if let Some(char) = self.state.characters.get_mut(char_id) {
            char.current_status = new_status;
        }
    }

    /// Record a new timeline event.
    pub fn add_timeline_event(&mut self, chapter: u32, description: String) {
        self.state.timeline.events.push(TimelineEvent {
            chapter,
            description,
        });
        self.state.timeline.current_chapter = chapter;
    }

    /// Add a new foreshadow.
    pub fn plant_foreshadow(&mut self, id: String, description: String, chapter: u32) {
        self.state.foreshadows.push(ForeshadowTracker {
            id,
            description,
            planted_at: chapter,
            status: ForeshadowStatus::Planted,
        });
    }

    /// Update foreshadow status.
    pub fn update_foreshadow_status(&mut self, foreshadow_id: &str, new_status: ForeshadowStatus) {
        if let Some(fh) = self.state.foreshadows.iter_mut().find(|f| f.id == foreshadow_id) {
            fh.status = new_status;
        }
    }

    /// Add a hard constraint.
    pub fn add_constraint(&mut self, id: String, description: String, severity: Severity) {
        self.state.hard_constraints.push(Constraint {
            id,
            description,
            severity,
        });
    }

    /// Set chapter goal.
    pub fn set_chapter_goal(&mut self, chapter: u32, description: String) {
        self.state.current_chapter_goal = Some(ChapterGoal {
            chapter,
            description,
        });
    }

    /// Clear chapter goal (after completion).
    pub fn clear_chapter_goal(&mut self) {
        self.state.current_chapter_goal = None;
    }

    /// Advance to next chapter.
    pub fn advance_chapter(&mut self) {
        self.state.meta.last_chapter += 1;
        self.state.timeline.current_chapter = self.state.meta.last_chapter;
        self.clear_chapter_goal();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    fn temp_path(tag: &str) -> std::path::PathBuf {
        let mut p = env::temp_dir();
        p.push(format!("na_story_helpers_test_{}_{}", tag, na_common::next_id("t")));
        p
    }

    #[test]
    fn add_character_works() {
        let path = temp_path("add_char");
        let mut mgr = StoryStateManager::open(&path).unwrap();

        let char = CharacterState {
            id: "c1".to_string(),
            name: "测试角色".to_string(),
            core_traits: vec!["勇敢".to_string()],
            current_status: "初始状态".to_string(),
            goals: vec![],
        };

        mgr.add_character(char);
        assert_eq!(mgr.state.characters.len(), 1);
        assert_eq!(mgr.state.characters.get("c1").unwrap().name, "测试角色");

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn timeline_progression() {
        let path = temp_path("timeline");
        let mut mgr = StoryStateManager::open(&path).unwrap();

        mgr.add_timeline_event(1, "事件1".to_string());
        mgr.add_timeline_event(2, "事件2".to_string());

        assert_eq!(mgr.state.timeline.events.len(), 2);
        assert_eq!(mgr.state.timeline.current_chapter, 2);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn foreshadow_lifecycle() {
        let path = temp_path("foreshadow");
        let mut mgr = StoryStateManager::open(&path).unwrap();

        mgr.plant_foreshadow("fh1".to_string(), "伏笔1".to_string(), 1);
        assert_eq!(mgr.state.foreshadows.len(), 1);
        assert_eq!(mgr.state.foreshadows[0].status, ForeshadowStatus::Planted);

        mgr.update_foreshadow_status("fh1", ForeshadowStatus::Hinted);
        assert_eq!(mgr.state.foreshadows[0].status, ForeshadowStatus::Hinted);

        mgr.update_foreshadow_status("fh1", ForeshadowStatus::Resolved);
        assert_eq!(mgr.state.foreshadows[0].status, ForeshadowStatus::Resolved);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn chapter_goal_management() {
        let path = temp_path("goal");
        let mut mgr = StoryStateManager::open(&path).unwrap();

        mgr.set_chapter_goal(1, "完成介绍".to_string());
        assert!(mgr.state.current_chapter_goal.is_some());

        mgr.clear_chapter_goal();
        assert!(mgr.state.current_chapter_goal.is_none());

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn advance_chapter_increments() {
        let path = temp_path("advance");
        let mut mgr = StoryStateManager::open(&path).unwrap();

        mgr.state.meta.last_chapter = 0;
        mgr.set_chapter_goal(1, "目标1".to_string());

        mgr.advance_chapter();
        assert_eq!(mgr.state.meta.last_chapter, 1);
        assert_eq!(mgr.state.timeline.current_chapter, 1);
        assert!(mgr.state.current_chapter_goal.is_none());

        std::fs::remove_file(&path).ok();
    }
}
