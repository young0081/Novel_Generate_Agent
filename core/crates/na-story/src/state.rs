//! Core data structures for story state management.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub type CharacterId = String;
pub type FactId = String;

/// Complete story state snapshot.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StoryState {
    /// Story metadata
    pub meta: StoryMeta,
    /// World setting (immutable truths)
    pub world: WorldState,
    /// Character states (mutable)
    pub characters: HashMap<CharacterId, CharacterState>,
    /// Current timeline position
    pub timeline: Timeline,
    /// Information possession matrix (who knows what)
    pub knowledge_matrix: KnowledgeMatrix,
    /// Foreshadowing tracking
    pub foreshadows: Vec<ForeshadowTracker>,
    /// Hard constraints (must not be violated)
    pub hard_constraints: Vec<Constraint>,
    /// Soft preferences (should follow when possible)
    pub soft_preferences: Vec<Preference>,
    /// Current chapter goal
    pub current_chapter_goal: Option<ChapterGoal>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StoryMeta {
    pub title: String,
    pub genre: String,
    pub last_chapter: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorldState {
    /// World rules (e.g., "Magic requires life force")
    pub rules: Vec<WorldRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorldRule {
    pub id: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CharacterState {
    pub id: CharacterId,
    pub name: String,
    /// Core traits (used for OOC detection)
    pub core_traits: Vec<String>,
    /// Current status
    pub current_status: String,
    /// Goals/motivations
    pub goals: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Timeline {
    /// Current time point (chapter number or absolute time)
    pub current_chapter: u32,
    /// Events that have occurred
    pub events: Vec<TimelineEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TimelineEvent {
    pub chapter: u32,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct KnowledgeMatrix {
    /// key: "character_id::fact_id"
    pub entries: HashMap<String, KnowledgeEntry>,
}

impl KnowledgeMatrix {
    pub fn new() -> Self {
        KnowledgeMatrix {
            entries: HashMap::new(),
        }
    }

    pub fn key(char_id: &str, fact_id: &str) -> String {
        format!("{}::{}", char_id, fact_id)
    }

    pub fn set_knowledge(&mut self, char_id: &str, fact_id: &str, knows: bool) {
        self.entries.insert(
            Self::key(char_id, fact_id),
            KnowledgeEntry {
                knows,
                learned_at: None,
            },
        );
    }

    pub fn knows(&self, char_id: &str, fact_id: &str) -> bool {
        self.entries
            .get(&Self::key(char_id, fact_id))
            .map(|e| e.knows)
            .unwrap_or(false)
    }
}

impl Default for KnowledgeMatrix {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct KnowledgeEntry {
    pub knows: bool,
    pub learned_at: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ForeshadowTracker {
    pub id: String,
    pub description: String,
    pub planted_at: u32,
    pub status: ForeshadowStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ForeshadowStatus {
    Planted,
    Hinted,
    Resolved,
    Abandoned,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Constraint {
    pub id: String,
    pub description: String,
    pub severity: Severity,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Info = 1,
    Low = 2,
    Medium = 3,
    High = 4,
    Critical = 5,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Preference {
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChapterGoal {
    pub chapter: u32,
    pub description: String,
}

impl Default for StoryState {
    fn default() -> Self {
        StoryState {
            meta: StoryMeta {
                title: String::new(),
                genre: String::new(),
                last_chapter: 0,
            },
            world: WorldState { rules: vec![] },
            characters: HashMap::new(),
            timeline: Timeline {
                current_chapter: 0,
                events: vec![],
            },
            knowledge_matrix: KnowledgeMatrix::new(),
            foreshadows: vec![],
            hard_constraints: vec![],
            soft_preferences: vec![],
            current_chapter_goal: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn story_state_serialization_roundtrip() {
        let mut state = StoryState::default();
        state.meta.title = "测试小说".to_string();

        let char_state = CharacterState {
            id: "char_001".to_string(),
            name: "主角".to_string(),
            core_traits: vec!["冷静".to_string(), "善良".to_string()],
            current_status: "初始状态".to_string(),
            goals: vec!["找到真相".to_string()],
        };
        state.characters.insert("char_001".to_string(), char_state);

        state.hard_constraints.push(Constraint {
            id: "hc_001".to_string(),
            description: "主角绝不会伤害无辜".to_string(),
            severity: Severity::Critical,
        });

        let json = serde_json::to_string(&state).unwrap();
        let loaded: StoryState = serde_json::from_str(&json).unwrap();
        assert_eq!(state, loaded);
    }

    #[test]
    fn knowledge_matrix_lookup() {
        let mut matrix = KnowledgeMatrix::new();
        matrix.set_knowledge("char_001", "fact_secret", true);
        assert!(matrix.knows("char_001", "fact_secret"));
        assert!(!matrix.knows("char_002", "fact_secret"));
    }

    #[test]
    fn severity_ordering() {
        assert!(Severity::Critical > Severity::High);
        assert!(Severity::High > Severity::Medium);
        assert!(Severity::Medium > Severity::Low);
        assert!(Severity::Low > Severity::Info);
    }
}
