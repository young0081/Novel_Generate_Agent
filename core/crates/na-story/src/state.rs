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
    #[serde(deserialize_with = "deserialize_characters")]
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
    legacy_text_id("rule", description)
}

fn legacy_text_id(kind: &str, description: &str) -> String {
    // Stable, dependency-free FNV-1a identifiers survive repeated legacy loads.
    let mut hash = 0x811c9dc5u32;
    for byte in description.as_bytes() {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(0x01000193);
    }
    format!("legacy_{kind}_{hash:08x}")
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

fn deserialize_characters<'de, D>(
    deserializer: D,
) -> Result<HashMap<CharacterId, CharacterState>, D::Error>
where
    D: Deserializer<'de>,
{
    let values = HashMap::<CharacterId, serde_json::Value>::deserialize(deserializer)?;
    values
        .into_iter()
        .map(|(key, mut value)| {
            if let Some(object) = value.as_object_mut() {
                object.entry("id").or_insert_with(|| key.clone().into());
                object.entry("name").or_insert_with(|| key.clone().into());
            }
            serde_json::from_value(value)
                .map(|character| (key, character))
                .map_err(serde::de::Error::custom)
        })
        .collect()
}

fn deserialize_text_list<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum TextList {
        Text(String),
        List(Vec<String>),
    }
    Ok(match Option::<TextList>::deserialize(deserializer)? {
        Some(TextList::Text(text)) => vec![text],
        Some(TextList::List(items)) => items,
        None => vec![],
    })
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
            #[serde(default, alias = "traits", deserialize_with = "deserialize_text_list")]
            core_traits: Vec<String>,
            #[serde(alias = "status")]
            current_status: Option<String>,
            #[serde(default, deserialize_with = "deserialize_text_list")]
            goals: Vec<String>,
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
                    core_traits: object.core_traits,
                    current_status,
                    goals: object.goals,
                })
            }
            _ => Err(serde::de::Error::custom(
                "character state must be a string or an object",
            )),
        }
    }
}

fn legacy_character_id(description: &str) -> String {
    legacy_text_id("character", description)
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
        self.entries
            .entry(Self::key(char_id, fact_id))
            .and_modify(|entry| entry.knows = Some(knows))
            .or_insert(KnowledgeEntry {
                knows: Some(knows),
                learned_at: None,
                description: None,
            });
    }

    pub fn knows(&self, char_id: &str, fact_id: &str) -> bool {
        self.entries
            .get(&Self::key(char_id, fact_id))
            .and_then(|e| e.knows)
            .unwrap_or(false)
    }
}

impl Default for KnowledgeMatrix {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct KnowledgeEntry {
    /// None means ownership is unspecified; a background note is not evidence
    /// that any character knows the fact. Existing true/false values stay intact.
    pub knows: Option<bool>,
    pub learned_at: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl<'de> Deserialize<'de> for KnowledgeEntry {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct EntryObject {
            knows: Option<bool>,
            learned_at: Option<u32>,
            description: Option<String>,
        }
        let value = serde_json::Value::deserialize(deserializer)?;
        let object = match value {
            serde_json::Value::String(description) => EntryObject {
                knows: None,
                learned_at: None,
                description: Some(description),
            },
            serde_json::Value::Bool(knows) => EntryObject {
                knows: Some(knows),
                learned_at: None,
                description: None,
            },
            serde_json::Value::Object(_) => {
                serde_json::from_value(value).map_err(serde::de::Error::custom)?
            }
            _ => {
                return Err(serde::de::Error::custom(
                    "knowledge entry must be a description string, boolean or object",
                ))
            }
        };
        if object.knows.is_none() && object.description.is_none() {
            return Err(serde::de::Error::custom(
                "knowledge entry needs knows or description",
            ));
        }
        Ok(Self {
            knows: object.knows,
            learned_at: object.learned_at,
            description: object.description,
        })
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ForeshadowTracker {
    pub id: String,
    pub description: String,
    pub planted_at: u32,
    pub status: ForeshadowStatus,
}

/// Legacy and model-authored descriptions have no chapter/status metadata.
/// Retain them as pending hints with an unknown planting chapter (zero).
impl<'de> Deserialize<'de> for ForeshadowTracker {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct ForeshadowObject {
            id: Option<String>,
            description: String,
            planted_at: Option<u32>,
            status: Option<ForeshadowStatus>,
        }

        let value = serde_json::Value::deserialize(deserializer)?;
        let object = match value {
            serde_json::Value::String(description) => ForeshadowObject {
                id: None,
                description,
                planted_at: None,
                status: None,
            },
            serde_json::Value::Object(_) => {
                serde_json::from_value(value).map_err(serde::de::Error::custom)?
            }
            _ => {
                return Err(serde::de::Error::custom(
                    "foreshadow must be a string or an object",
                ))
            }
        };
        Ok(Self {
            id: object
                .id
                .unwrap_or_else(|| legacy_text_id("foreshadow", &object.description)),
            description: object.description,
            planted_at: object.planted_at.unwrap_or_default(),
            status: object.status.unwrap_or(ForeshadowStatus::Planted),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ForeshadowStatus {
    Planted,
    Hinted,
    Resolved,
    Abandoned,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Constraint {
    pub id: String,
    pub description: String,
    pub severity: Severity,
}

/// Keep plain-text hard constraints active in the next chapter's context.
impl<'de> Deserialize<'de> for Constraint {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct ConstraintObject {
            id: Option<String>,
            description: String,
            severity: Option<Severity>,
        }

        let value = serde_json::Value::deserialize(deserializer)?;
        let object = match value {
            serde_json::Value::String(description) => ConstraintObject {
                id: None,
                description,
                severity: None,
            },
            serde_json::Value::Object(_) => {
                serde_json::from_value(value).map_err(serde::de::Error::custom)?
            }
            _ => {
                return Err(serde::de::Error::custom(
                    "constraint must be a string or an object",
                ))
            }
        };
        Ok(Self {
            id: object
                .id
                .unwrap_or_else(|| legacy_text_id("constraint", &object.description)),
            description: object.description,
            severity: object.severity.unwrap_or(Severity::High),
        })
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Info = 1,
    Low = 2,
    Medium = 3,
    High = 4,
    Critical = 5,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Preference {
    pub description: String,
}

impl<'de> Deserialize<'de> for Preference {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct PreferenceObject {
            description: String,
        }

        let value = serde_json::Value::deserialize(deserializer)?;
        match value {
            serde_json::Value::String(description) => Ok(Self { description }),
            serde_json::Value::Object(_) => {
                let object: PreferenceObject =
                    serde_json::from_value(value).map_err(serde::de::Error::custom)?;
                Ok(Self {
                    description: object.description,
                })
            }
            _ => Err(serde::de::Error::custom(
                "preference must be a string or an object",
            )),
        }
    }
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
    fn legacy_text_lists_preserve_content_and_structured_metadata() {
        let mut json = serde_json::to_value(StoryState::default()).unwrap();
        json["foreshadows"] = serde_json::json!([
            "玄机子留给顾长生的生锈铜钥匙及洞府试炼",
            {"id": "resolved_key", "description": "旧钥匙", "planted_at": 1, "status": "Resolved"},
            {"id": "hinted_key", "description": "新线索", "planted_at": 2, "status": "Hinted"},
            {"id": "abandoned_key", "description": "废弃线索", "planted_at": 1, "status": "Abandoned"}
        ]);
        json["hard_constraints"] = serde_json::json!([
            "主角不能凭空知道秘密",
            {"id": "critical_secret", "description": "身份保密", "severity": "Critical"}
        ]);
        json["soft_preferences"] = serde_json::json!([
            "对话简洁",
            {"description": "少用旁白"}
        ]);

        let state: StoryState = serde_json::from_value(json.clone()).unwrap();
        let reloaded: StoryState = serde_json::from_value(json).unwrap();
        assert_eq!(state, reloaded);
        assert_eq!(
            state.foreshadows[0].description,
            "玄机子留给顾长生的生锈铜钥匙及洞府试炼"
        );
        assert_eq!(state.foreshadows[0].planted_at, 0);
        assert_eq!(state.foreshadows[0].status, ForeshadowStatus::Planted);
        assert!(state.foreshadows[0].id.starts_with("legacy_foreshadow_"));
        assert_eq!(state.foreshadows[1].id, "resolved_key");
        assert_eq!(state.foreshadows[1].status, ForeshadowStatus::Resolved);
        assert_eq!(state.foreshadows[2].planted_at, 2);
        assert_eq!(state.foreshadows[2].status, ForeshadowStatus::Hinted);
        assert_eq!(state.foreshadows[3].status, ForeshadowStatus::Abandoned);
        assert_eq!(
            state.hard_constraints[0].description,
            "主角不能凭空知道秘密"
        );
        assert_eq!(state.hard_constraints[0].severity, Severity::High);
        assert!(state.hard_constraints[0]
            .id
            .starts_with("legacy_constraint_"));
        assert_eq!(state.hard_constraints[1].severity, Severity::Critical);
        assert_eq!(state.soft_preferences[0].description, "对话简洁");
        assert_eq!(state.soft_preferences[1].description, "少用旁白");

        let saved = serde_json::to_value(&state).unwrap();
        for field in ["foreshadows", "hard_constraints", "soft_preferences"] {
            assert!(saved[field]
                .as_array()
                .unwrap()
                .iter()
                .all(|item| item.is_object()));
        }
        assert_eq!(state, serde_json::from_value(saved).unwrap());
    }

    #[test]
    fn malformed_text_lists_are_not_silently_discarded() {
        for field in ["foreshadows", "hard_constraints", "soft_preferences"] {
            for invalid in [
                serde_json::json!(42),
                serde_json::json!(null),
                serde_json::json!({}),
                serde_json::json!({"description": false}),
            ] {
                let mut json = serde_json::to_value(StoryState::default()).unwrap();
                json[field] = serde_json::json!([invalid]);
                assert!(
                    serde_json::from_value::<StoryState>(json).is_err(),
                    "{field}"
                );
            }
        }
        assert!(
            serde_json::from_value::<ForeshadowTracker>(serde_json::json!({
                "id": "f", "description": "key", "planted_at": 1, "status": "unknown"
            }))
            .is_err()
        );
        assert!(serde_json::from_value::<Constraint>(serde_json::json!({
            "id": "c", "description": "secret", "severity": "unknown"
        }))
        .is_err());
    }

    #[test]
    fn knowledge_entries_preserve_notes_ownership_and_learned_chapters() {
        let value = serde_json::json!({"entries": {
            "place": "旧列车的坠落地点",
            "hero::route": {"knows": true, "learned_at": 2, "description": "向北"},
            "rival::route": {"knows": false, "learned_at": null},
            "friend::route": false,
            "guide::route": true,
            "observer::route": {"description": "可能看过地图"}
        }});
        let mut matrix: KnowledgeMatrix = serde_json::from_value(value).unwrap();
        assert_eq!(matrix.entries["place"].knows, None);
        assert_eq!(
            matrix.entries["place"].description.as_deref(),
            Some("旧列车的坠落地点")
        );
        assert_eq!(matrix.entries["hero::route"].learned_at, Some(2));
        assert!(matrix.knows("hero", "route"));
        assert!(matrix.knows("guide", "route"));
        assert!(!matrix.knows("rival", "route"));
        assert!(!matrix.knows("friend", "route"));
        assert!(!matrix.knows("observer", "route"));
        matrix.set_knowledge("hero", "route", false);
        assert_eq!(
            matrix.entries["hero::route"].description.as_deref(),
            Some("向北")
        );
        assert_eq!(matrix.entries["hero::route"].learned_at, Some(2));
        let saved = serde_json::to_value(&matrix).unwrap();
        assert_eq!(saved["entries"]["rival::route"]["knows"], false);
        assert!(saved["entries"]["place"]["knows"].is_null());
        assert_eq!(matrix, serde_json::from_value(saved).unwrap());
    }

    #[test]
    fn malformed_knowledge_and_character_fields_are_not_silently_discarded() {
        for value in [
            serde_json::json!(null),
            serde_json::json!(17),
            serde_json::json!([]),
            serde_json::json!({}),
            serde_json::json!({"knows": "false"}),
            serde_json::json!({"description": 17}),
            serde_json::json!({"knows": true, "learned_at": "yesterday"}),
        ] {
            assert!(serde_json::from_value::<KnowledgeEntry>(value).is_err());
        }
        for value in [
            serde_json::json!({"status": 17}),
            serde_json::json!({"traits": [false]}),
            serde_json::json!({"goals": {"hidden": "goal"}}),
        ] {
            assert!(serde_json::from_value::<CharacterState>(value).is_err());
        }
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
