//! Knowledge base tools — let the agent save facts into a work's knowledge base
//! during a run, so a "web-fill" goal can research canon material and write
//! structured entries without the UI needing to parse JSON.

use na_common::{CoreError, Result};
use serde_json::{json, Value as Json};

use crate::tool::{Capability, Tool, ToolContext, ToolResult};

/// Save a knowledge entry into the active work's currently-selected knowledge base.
///
/// The agent running a "web-fill" goal can call this repeatedly to write canon
/// facts it discovers via http_get / web search. The `kb_id` must be passed by
/// the goal's system prompt (the UI sets up the run with the target KB), or the
/// tool will fail with a "no active KB" error.
///
/// Input schema:
/// ```json
/// {
///   "kb_id": "kb_...",
///   "kind": "character|location|worldbuilding|event|item|term|lore|other",
///   "title": "短标题",
///   "content": "详细设定",
///   "tags": ["tag1", "tag2"],
///   "source": "https://... or 'web' or 'ai'"
/// }
/// ```
pub struct KnowledgeSave;

impl Tool for KnowledgeSave {
    fn name(&self) -> &str {
        "knowledge_save"
    }

    fn description(&self) -> &str {
        "将一条设定资料保存到当前作品的知识库中。用于联网填充或研究时记录角色、地点、\
        世界规则、事件、器物、术语等设定信息。kind 可选值: character, location, \
        worldbuilding, event, item, term, lore, other。返回保存的条目 ID。"
    }

    fn input_schema(&self) -> Json {
        json!({
            "type": "object",
            "properties": {
                "kb_id": {
                    "type": "string",
                    "description": "目标知识库的 ID（由系统提示提供）"
                },
                "kind": {
                    "type": "string",
                    "enum": ["character", "location", "worldbuilding", "event", "item", "term", "lore", "other"],
                    "description": "条目类型"
                },
                "title": {
                    "type": "string",
                    "description": "短标题（如人名、地名、术语）"
                },
                "content": {
                    "type": "string",
                    "description": "详细设定描述"
                },
                "tags": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "标签列表（可选）"
                },
                "source": {
                    "type": "string",
                    "description": "来源（URL 或 'web' / 'ai'）"
                }
            },
            "required": ["kb_id", "kind", "title", "content"]
        })
    }

    fn capabilities(&self) -> &[Capability] {
        &[Capability::Write]
    }

    fn is_mutating(&self) -> bool {
        true
    }

    fn execute(&self, args: Json, ctx: &ToolContext) -> Result<ToolResult> {
        let kb_id = args
            .get("kb_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| CoreError::invalid_input("missing kb_id"))?;
        let kind_str = args
            .get("kind")
            .and_then(|v| v.as_str())
            .ok_or_else(|| CoreError::invalid_input("missing kind"))?;
        let title = args
            .get("title")
            .and_then(|v| v.as_str())
            .ok_or_else(|| CoreError::invalid_input("missing title"))?;
        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| CoreError::invalid_input("missing content"))?;
        let tags: Vec<String> = args
            .get("tags")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        let source = args
            .get("source")
            .and_then(|v| v.as_str())
            .unwrap_or("agent");

        // The knowledge directory lives under the workspace root. The UI's
        // setup phase created it as `workspace/../knowledge` (sibling to
        // workspace), or we fail gracefully if it's missing.
        let workspace = ctx.jail.root();
        let knowledge_dir = workspace.parent().and_then(|p| {
            let d = p.join("knowledge");
            if d.exists() { Some(d) } else { None }
        }).ok_or_else(|| CoreError::not_found(
            "knowledge directory not found (only works in multi-work mode)"
        ))?;

        // Dynamically load na-library (avoid circular dep at build time).
        use std::path::PathBuf;
        let store = open_knowledge_store(&knowledge_dir)?;
        let mut kb = open_kb(&store, kb_id)?;
        let kind = parse_kind(kind_str);
        let entry_id = add_entry(&mut kb, kind, title, content, source, tags)?;

        Ok(ToolResult::ok(format!("已保存设定条目「{title}」(ID: {entry_id})")).with_data(json!({
            "entry_id": entry_id,
            "kb_id": kb_id
        })))
    }
}

// ---- dynamic dispatch to na-library (avoids circular dep at link time) ----

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KnowledgeKind {
    Character,
    Location,
    Worldbuilding,
    Event,
    Item,
    Term,
    Lore,
    Other,
}

fn parse_kind(s: &str) -> KnowledgeKind {
    match s {
        "character" => KnowledgeKind::Character,
        "location" => KnowledgeKind::Location,
        "worldbuilding" => KnowledgeKind::Worldbuilding,
        "event" => KnowledgeKind::Event,
        "item" => KnowledgeKind::Item,
        "term" => KnowledgeKind::Term,
        "lore" => KnowledgeKind::Lore,
        _ => KnowledgeKind::Other,
    }
}

// Minimal shim to na-library types; we only link at runtime via the caller's dep.
fn open_knowledge_store(dir: &std::path::Path) -> Result<Box<dyn std::any::Any>> {
    // This would be `na_library::KnowledgeStore::open(dir)`, but to avoid
    // na-tools depending on na-library (circular), we fake it with reflection.
    // The real solution is to make the Tauri command itself inject this tool
    // with a closure that captures the KnowledgeStore, rather than baking it
    // into na-tools.
    //
    // For now, return a placeholder error — the actual integration happens in
    // the Tauri layer where both na-library and na-tools are available.
    Err(CoreError::internal(
        "knowledge_save tool requires integration at the host layer"
    ))
}

fn open_kb(_store: &Box<dyn std::any::Any>, _kb_id: &str) -> Result<Box<dyn std::any::Any>> {
    Err(CoreError::internal(
        "knowledge_save tool requires integration at the host layer"
    ))
}

fn add_entry(
    _kb: &mut Box<dyn std::any::Any>,
    _kind: KnowledgeKind,
    _title: &str,
    _content: &str,
    _source: &str,
    _tags: Vec<String>,
) -> Result<String> {
    Err(CoreError::internal(
        "knowledge_save tool requires integration at the host layer"
    ))
}
