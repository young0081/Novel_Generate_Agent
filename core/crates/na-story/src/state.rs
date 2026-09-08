//! Core data structures for story state management.

use serde::{Deserialize, Deserializer, Serialize};
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

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct WorldRule {
    pub id: String,
    pub description: String,
}

/// Accept the pre-0.3 story-state format where a world rule was stored as a
/// bare string. New saves always serialize the structured representation.
impl<'de> Deserialize<'de> for WorldRule {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct WorldRuleObject {
            id: Option<String>,
            description: Option<String>,
        }

        let value = serde_json::Value::deserialize(deserializer)?;
        match value {
            serde_json::Value::String(description) => Ok(Self {
                id: legacy_rule_id(&description),
                description,
            }),
            serde_json::Value::Object(_) => {
                let object: WorldRuleObject =
                    serde_json::from_value(value).map_err(serde::de::Error::custom)?;
                let description = object
                    .description
                    .ok_or_else(|| serde::de::Error::missing_field("description"))?;
                Ok(Self {
                    id: object.id.unwrap_or_else(|| legacy_rule_id(&description)),
                    description,
                })
            }
            _ => Err(serde::de::Error::custom(
                "world rule must be a string or an object",
            )),
        }
    }
}

fn legacy_rule_id(description: &str) -> String {
    // Stable, dependency-free FNV-1a identifier for migrated legacy rules.
    let mut hash = 0x811c9dc5u32;
    for byte in description.as_bytes() {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(0x01000193);
    }
    format!("legacy_rule_{hash:08x}")
}

#[derive(Debug, Clone, Serialize, PartialEq)]
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

/// Accept the pre-0.3 story-state format where a character was stored as a
/// plain description string. New saves always serialize the structured form.
impl<'de> Deserialize<'de> for CharacterState {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct CharacterStateObject {
            id: Option<CharacterId>,
            name: Option<String>,
            core_traits: Option<Vec<String>>,
            current_status: Option<String>,
            goals: Option<Vec<String>>,
        }

        let value = serde_json::Value::deserialize(deserializer)?;
        match value {
            serde_json::Value::String(description) => Ok(Self {
                id: legacy_character_id(&description),
                name: legacy_character_name(&description),
                core_traits: vec![],
                current_status: description,
                goals: vec![],
            }),
            serde_json::Value::Object(_) => {
                let object: CharacterStateObject =
                    serde_json::from_value(value).map_err(serde::de::Error::custom)?;
                let name = object.name.unwrap_or_default();
                let current_status = object.current_status.unwrap_or_default();
                let id = object.id.unwrap_or_else(|| {
                    let seed = if name.is_empty() {
                        current_status.clone()
                    } else {
                        format!("{name}|{current_status}")
                    };
                    legacy_character_id(&seed)
                });
                Ok(Self {
                    id,
                    name,
                    core_traits: object.core_traits.unwrap_or_default(),
                    current_status,
                    goals: object.goals.unwrap_or_default(),
                })
            }
            _ => Err(serde::de::Error::custom(
                "character state must be a string or an object",
            )),
        }
    }
}

fn legacy_character_id(description: &str) -> String {
    // Stable, dependency-free FNV-1a identifier for migrated legacy characters.
    let mut hash = 0x811c9dc5u32;
    for byte in description.as_bytes() {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(0x01000193);
    }
    format!("legacy_character_{hash:08x}")
}

fn legacy_character_name(description: &str) -> String {
    description
        .split(|character: char| {
            matches!(
                character,
                '，' | ',' | '。' | '.' | '；' | ';' | ':' | '：' | '\n'
            )
        })
        .next()
        .unwrap_or(description)
        .trim()
        .to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Timeline {
    /// Current time point (chapter number or absolute time)
    pub current_chapter: u32,
    /// Events that have occurred
    pub events: Vec<TimelineEvent>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TimelineEvent {
    pub chapter: u32,
    pub description: String,
}

/// Accept the pre-0.3 story-state format where a timeline event was stored as
/// a bare description string. The chapter is unknown in that format and is
/// represented as zero until the user or a later save assigns it.
impl<'de> Deserialize<'de> for TimelineEvent {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct TimelineEventObject {
            chapter: Option<u32>,
            description: Option<String>,
        }

        let value = serde_json::Value::deserialize(deserializer)?;
        match value {
            serde_json::Value::String(description) => Ok(Self {
                chapter: 0,
                description,
            }),
            serde_json::Value::Object(_) => {
                let object: TimelineEventObject =
                    serde_json::from_value(value).map_err(serde::de::Error::custom)?;
                Ok(Self {
                    chapter: object.chapter.unwrap_or_default(),
                    description: object
                        .description
                        .ok_or_else(|| serde::de::Error::missing_field("description"))?,
                })
            }
            _ => Err(serde::de::Error::custom(
                "timeline event must be a string or an object",
            )),
        }
    }
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
    fn legacy_string_state_fields_are_migrated_on_load() {
        let json = serde_json::json!({
            "meta": {"title": "旧作品", "genre": "修真", "last_chapter": 2},
            "world": {"rules": [
                "天道师父会自觉兜底，但底层代码残缺频繁掉血",
                {"id": "wr_new", "description": "灵气不足时法术会衰减"}
            ]},
            "characters": {
                "legacy_master": "师尊，身合天道的修仙界大能，化为天道后仍在暗中护持主角",
                "c_new": {
                    "id": "c_new",
                    "name": "主角",
                    "core_traits": ["坚韧"],
                    "current_status": "初入修行",
                    "goals": ["寻找真相"]
                }
            },
            "timeline": {"current_chapter": 2, "events": [
                "玄门师尊合道，天道开始暗中护持主角",
                {"chapter": 1, "description": "主角踏入修行"}
            ]},
            "knowledge_matrix": {"entries": {}},
            "foreshadows": [],
            "hard_constraints": [],
            "soft_preferences": [],
            "current_chapter_goal": null
        });
        let state: StoryState = serde_json::from_value(json).unwrap();
        assert_eq!(state.world.rules.len(), 2);
        assert_eq!(
            state.world.rules[0].description,
            "天道师父会自觉兜底，但底层代码残缺频繁掉血"
        );
        assert!(state.world.rules[0].id.starts_with("legacy_rule_"));
        assert_eq!(state.world.rules[1].id, "wr_new");
        let legacy_character = &state.characters["legacy_master"];
        assert_eq!(legacy_character.name, "师尊");
        assert_eq!(
            legacy_character.current_status,
            "师尊，身合天道的修仙界大能，化为天道后仍在暗中护持主角"
        );
        assert!(legacy_character.id.starts_with("legacy_character_"));
        assert_eq!(state.characters["c_new"].id, "c_new");
        assert_eq!(state.timeline.events[0].chapter, 0);
        assert_eq!(
            state.timeline.events[0].description,
            "玄门师尊合道，天道开始暗中护持主角"
        );
        assert_eq!(state.timeline.events[1].chapter, 1);
        let saved = serde_json::to_value(&state).unwrap();
        assert!(saved["world"]["rules"][0].is_object());
        assert!(saved["characters"]["legacy_master"].is_object());
        assert!(saved["timeline"]["events"][0].is_object());

        let reloaded: StoryState = serde_json::from_value(saved).unwrap();
        assert_eq!(reloaded.characters["legacy_master"].id, legacy_character.id);
        assert_eq!(reloaded.characters["c_new"], state.characters["c_new"]);
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
