//! Tools wrapping the long-term memory and checkpoint stores held in the
//! [`ToolContext`].
//!
//! These tools let the agent persist and recall story facts and snapshot the
//! workspace, all through the standard tool protocol:
//!
//! * [`MemorySaveTool`] — save a memory (cap [`WriteMemory`]).
//! * [`MemoryRecallTool`] — recall relevant memories, returning only structured
//!   [`RecallHit`](na_memory::RecallHit) headers, never full content
//!   (cap [`ReadMemory`]).
//! * [`MemoryClassifyTool`] — reclassify / tag a memory (cap [`WriteMemory`]).
//! * [`MemoryArchiveTool`] — archive or un-archive a memory (cap [`WriteMemory`]).
//! * [`CheckpointCreateTool`] / [`CheckpointListTool`] / [`CheckpointRestoreTool`]
//!   — workspace snapshots wrapping the [`CheckpointStore`](na_memory::CheckpointStore).

use na_common::{json, CoreError, Json, MemoryId, Result};
use na_memory::{CheckpointId, MemoryEntry, MemoryKind};
use na_sandbox::Capability;

use crate::tool::{BoxFuture, Tool, ToolContext, ToolResult, ToolSpec};

/// Parse a [`MemoryKind`] from a snake_case string.
fn parse_kind(s: &str) -> Result<MemoryKind> {
    serde_json::from_value::<MemoryKind>(Json::String(s.to_string()))
        .map_err(|_| CoreError::invalid_input(format!("unknown memory kind {s:?}")))
}

/// Convert an array-of-strings JSON value into `Vec<String>`.
fn string_vec(value: Option<&Json>) -> Vec<String> {
    match value {
        Some(Json::Array(arr)) => arr
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect(),
        _ => Vec::new(),
    }
}

/// Save a new long-term memory.
#[derive(Debug, Clone, Copy, Default)]
pub struct MemorySaveTool;

impl Tool for MemorySaveTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "memory_save",
            "Save a durable story memory (character, setting, plot, foreshadow, ...). \
             Returns the new memory id.",
            json!({
                "type": "object",
                "required": ["kind", "title", "summary", "content"],
                "properties": {
                    "kind": { "enum": ["character","setting","worldbuilding","plot","outline","foreshadow","dialogue","lore","other"] },
                    "title": { "type": "string", "minLength": 1 },
                    "summary": { "type": "string" },
                    "content": { "type": "string" },
                    "tags": { "type": "array", "items": { "type": "string" } },
                    "importance": { "type": "integer", "minimum": 1, "maximum": 5 }
                },
                "additionalProperties": false
            }),
            vec![Capability::WriteMemory],
            true,
        )
    }

    fn execute<'a>(
        &'a self,
        args: Json,
        ctx: &'a ToolContext,
    ) -> BoxFuture<'a, Result<ToolResult>> {
        Box::pin(async move {
            let kind = parse_kind(require_str(&args, "kind")?)?;
            let title = require_str(&args, "title")?.to_string();
            let summary = args
                .get("summary")
                .and_then(Json::as_str)
                .unwrap_or("")
                .to_string();
            let content = args
                .get("content")
                .and_then(Json::as_str)
                .unwrap_or("")
                .to_string();
            let tags = string_vec(args.get("tags"));
            let importance = args.get("importance").and_then(Json::as_u64).unwrap_or(3) as u8;

            let id = {
                let mut store = ctx
                    .memory
                    .lock()
                    .map_err(|_| CoreError::internal("memory store lock poisoned"))?;
                store.save(kind, title.clone(), summary, content, tags, importance)?
            };
            Ok(ToolResult::success(
                format!("saved memory {} ({title})", id.as_str()),
                json!({ "id": id.as_str(), "title": title }),
            )
            .with_summary(format!("saved memory {}", id.as_str())))
        })
    }
}

/// Recall relevant memories (structured headers only).
#[derive(Debug, Clone, Copy, Default)]
pub struct MemoryRecallTool;

impl Tool for MemoryRecallTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "memory_recall",
            "Recall the most relevant story memories for a query. Returns only structured \
             headers (title/summary/tags/score), never full content.",
            json!({
                "type": "object",
                "required": ["query"],
                "properties": {
                    "query": { "type": "string", "minLength": 1 },
                    "k": { "type": "integer", "minimum": 1, "maximum": 50 },
                    "kind": { "enum": ["character","setting","worldbuilding","plot","outline","foreshadow","dialogue","lore","other"] },
                    "include_archived": { "type": "boolean" }
                },
                "additionalProperties": false
            }),
            vec![Capability::ReadMemory],
            false,
        )
    }

    fn execute<'a>(
        &'a self,
        args: Json,
        ctx: &'a ToolContext,
    ) -> BoxFuture<'a, Result<ToolResult>> {
        Box::pin(async move {
            let query = require_str(&args, "query")?;
            let k = args.get("k").and_then(Json::as_u64).unwrap_or(5) as usize;
            let kind = match args.get("kind").and_then(Json::as_str) {
                Some(s) => Some(parse_kind(s)?),
                None => None,
            };
            let include_archived = args
                .get("include_archived")
                .and_then(Json::as_bool)
                .unwrap_or(false);

            let hits = {
                let store = ctx
                    .memory
                    .lock()
                    .map_err(|_| CoreError::internal("memory store lock poisoned"))?;
                store.recall(query, k, kind, include_archived)
            };

            let mut text = String::new();
            for h in &hits {
                text.push_str(&format!(
                    "[{}] {} — {} (importance {})\n",
                    h.id.as_str(),
                    h.title,
                    h.summary,
                    h.importance
                ));
            }
            if text.is_empty() {
                text.push_str("(no matching memories)");
            }
            let value = serde_json::to_value(&hits)?;
            Ok(
                ToolResult::success(text, json!({ "hits": value, "count": hits.len() }))
                    .with_summary(format!("{} hit(s)", hits.len())),
            )
        })
    }
}

/// List stored memories (optionally filtered by kind) — for browsing, not searching.
///
/// Unlike [`MemoryRecallTool`] (a top-k relevance search that needs a query),
/// this returns ALL matching entries' structured headers, importance-first, so a
/// UI can show the full set of e.g. every character. Full content is still only
/// fetched on demand.
#[derive(Debug, Clone, Copy, Default)]
pub struct MemoryListTool;

impl Tool for MemoryListTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "memory_list",
            "List stored memories (optionally filtered by one or more kinds), most important first. \
             Returns structured headers only (never full content). Use this to browse a whole \
             category; use memory_recall to search by relevance.",
            json!({
                "type": "object",
                "properties": {
                    "kind": { "enum": ["character","setting","worldbuilding","plot","outline","foreshadow","dialogue","lore","other"] },
                    "kinds": { "type": "array", "items": { "enum": ["character","setting","worldbuilding","plot","outline","foreshadow","dialogue","lore","other"] } },
                    "include_archived": { "type": "boolean" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 1000 }
                },
                "additionalProperties": false
            }),
            vec![Capability::ReadMemory],
            false,
        )
    }

    fn execute<'a>(
        &'a self,
        args: Json,
        ctx: &'a ToolContext,
    ) -> BoxFuture<'a, Result<ToolResult>> {
        Box::pin(async move {
            // Collect the kind filter from `kind` and/or `kinds`.
            let mut kinds: Vec<MemoryKind> = Vec::new();
            if let Some(s) = args.get("kind").and_then(Json::as_str) {
                kinds.push(parse_kind(s)?);
            }
            if let Some(arr) = args.get("kinds").and_then(Json::as_array) {
                for v in arr {
                    if let Some(s) = v.as_str() {
                        kinds.push(parse_kind(s)?);
                    }
                }
            }
            let include_archived = args
                .get("include_archived")
                .and_then(Json::as_bool)
                .unwrap_or(false);
            let limit = args.get("limit").and_then(Json::as_u64).unwrap_or(200) as usize;

            let (items, total): (Vec<Json>, usize) = {
                let store = ctx
                    .memory
                    .lock()
                    .map_err(|_| CoreError::internal("memory store lock poisoned"))?;
                let mut selected: Vec<&MemoryEntry> = store
                    .all()
                    .iter()
                    .filter(|e| include_archived || !e.archived)
                    .filter(|e| kinds.is_empty() || kinds.contains(&e.kind))
                    .collect();
                // Most important first, then most recently updated.
                selected.sort_by(|a, b| {
                    b.importance
                        .cmp(&a.importance)
                        .then(b.updated_ms.cmp(&a.updated_ms))
                });
                let total = selected.len();
                let items = selected
                    .into_iter()
                    .take(limit)
                    .map(|e| {
                        json!({
                            "id": e.id.as_str(),
                            "kind": e.kind,
                            "title": e.title,
                            "summary": e.summary,
                            "tags": e.tags,
                            "importance": e.importance,
                            "archived": e.archived,
                        })
                    })
                    .collect();
                (items, total)
            };

            let mut text = String::new();
            for it in &items {
                text.push_str(&format!(
                    "[{}] {} — {} (importance {})\n",
                    it["id"].as_str().unwrap_or(""),
                    it["title"].as_str().unwrap_or(""),
                    it["summary"].as_str().unwrap_or(""),
                    it["importance"].as_u64().unwrap_or(0),
                ));
            }
            if text.is_empty() {
                text.push_str("(no memories)");
            }
            Ok(ToolResult::success(
                text,
                json!({ "entries": items, "count": items.len(), "total": total }),
            )
            .with_summary(format!("{} memory(ies)", items.len())))
        })
    }
}

/// Reclassify a memory and/or add tags.
#[derive(Debug, Clone, Copy, Default)]
pub struct MemoryClassifyTool;

impl Tool for MemoryClassifyTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "memory_classify",
            "Change a memory's kind and/or append tags.",
            json!({
                "type": "object",
                "required": ["id"],
                "properties": {
                    "id": { "type": "string", "minLength": 1 },
                    "kind": { "enum": ["character","setting","worldbuilding","plot","outline","foreshadow","dialogue","lore","other"] },
                    "add_tags": { "type": "array", "items": { "type": "string" } }
                },
                "additionalProperties": false
            }),
            vec![Capability::WriteMemory],
            true,
        )
    }

    fn execute<'a>(
        &'a self,
        args: Json,
        ctx: &'a ToolContext,
    ) -> BoxFuture<'a, Result<ToolResult>> {
        Box::pin(async move {
            let id = MemoryId::from_existing(require_str(&args, "id")?);
            let kind = match args.get("kind").and_then(Json::as_str) {
                Some(s) => Some(parse_kind(s)?),
                None => None,
            };
            let add_tags = string_vec(args.get("add_tags"));
            {
                let mut store = ctx
                    .memory
                    .lock()
                    .map_err(|_| CoreError::internal("memory store lock poisoned"))?;
                store.classify(&id, kind, add_tags)?;
            }
            Ok(ToolResult::success(
                format!("classified {}", id.as_str()),
                json!({ "id": id.as_str() }),
            ))
        })
    }
}

/// Archive or un-archive a memory.
#[derive(Debug, Clone, Copy, Default)]
pub struct MemoryArchiveTool;

impl Tool for MemoryArchiveTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "memory_archive",
            "Archive (hide from recall) or un-archive a memory.",
            json!({
                "type": "object",
                "required": ["id", "archived"],
                "properties": {
                    "id": { "type": "string", "minLength": 1 },
                    "archived": { "type": "boolean" }
                },
                "additionalProperties": false
            }),
            vec![Capability::WriteMemory],
            true,
        )
    }

    fn execute<'a>(
        &'a self,
        args: Json,
        ctx: &'a ToolContext,
    ) -> BoxFuture<'a, Result<ToolResult>> {
        Box::pin(async move {
            let id = MemoryId::from_existing(require_str(&args, "id")?);
            let archived = args
                .get("archived")
                .and_then(Json::as_bool)
                .ok_or_else(|| CoreError::invalid_input("missing boolean argument \"archived\""))?;
            {
                let mut store = ctx
                    .memory
                    .lock()
                    .map_err(|_| CoreError::internal("memory store lock poisoned"))?;
                store.archive(&id, archived)?;
            }
            Ok(ToolResult::success(
                format!("set archived={archived} on {}", id.as_str()),
                json!({ "id": id.as_str(), "archived": archived }),
            ))
        })
    }
}

/// Permanently delete a memory entry.
#[derive(Debug, Clone, Copy, Default)]
pub struct MemoryDeleteTool;

impl Tool for MemoryDeleteTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "memory_delete",
            "Permanently delete a memory entry by id.",
            json!({
                "type": "object",
                "required": ["id"],
                "properties": { "id": { "type": "string", "minLength": 1 } },
                "additionalProperties": false
            }),
            vec![Capability::WriteMemory],
            true,
        )
    }

    fn execute<'a>(
        &'a self,
        args: Json,
        ctx: &'a ToolContext,
    ) -> BoxFuture<'a, Result<ToolResult>> {
        Box::pin(async move {
            let id = MemoryId::from_existing(require_str(&args, "id")?);
            {
                let mut store = ctx
                    .memory
                    .lock()
                    .map_err(|_| CoreError::internal("memory store lock poisoned"))?;
                store.delete(&id)?;
            }
            Ok(ToolResult::success(
                format!("deleted memory {}", id.as_str()),
                json!({ "id": id.as_str(), "deleted": true }),
            ))
        })
    }
}

/// Create a workspace checkpoint (snapshot).
#[derive(Debug, Clone, Copy, Default)]
pub struct CheckpointCreateTool;

impl Tool for CheckpointCreateTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "checkpoint_create",
            "Take a labelled byte-exact snapshot of the whole workspace.",
            json!({
                "type": "object",
                "required": ["label"],
                "properties": { "label": { "type": "string", "minLength": 1 } },
                "additionalProperties": false
            }),
            vec![Capability::WriteFile],
            true,
        )
    }

    fn execute<'a>(
        &'a self,
        args: Json,
        ctx: &'a ToolContext,
    ) -> BoxFuture<'a, Result<ToolResult>> {
        Box::pin(async move {
            let label = require_str(&args, "label")?;
            let id = {
                let mut store = ctx
                    .checkpoints
                    .lock()
                    .map_err(|_| CoreError::internal("checkpoint store lock poisoned"))?;
                store.create(label)?
            };
            Ok(ToolResult::success(
                format!("created checkpoint {} ({label})", id.as_str()),
                json!({ "id": id.as_str(), "label": label }),
            )
            .with_summary(format!("checkpoint {}", id.as_str())))
        })
    }
}

/// List all workspace checkpoints.
#[derive(Debug, Clone, Copy, Default)]
pub struct CheckpointListTool;

impl Tool for CheckpointListTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "checkpoint_list",
            "List workspace checkpoints (oldest first) with labels and file counts.",
            json!({ "type": "object", "properties": {}, "additionalProperties": false }),
            vec![],
            false,
        )
    }

    fn execute<'a>(
        &'a self,
        _args: Json,
        ctx: &'a ToolContext,
    ) -> BoxFuture<'a, Result<ToolResult>> {
        Box::pin(async move {
            let metas = {
                let store = ctx
                    .checkpoints
                    .lock()
                    .map_err(|_| CoreError::internal("checkpoint store lock poisoned"))?;
                store.list()
            };
            let mut text = String::new();
            for m in &metas {
                text.push_str(&format!(
                    "{} | {} | {} file(s)\n",
                    m.id.as_str(),
                    m.label,
                    m.file_count
                ));
            }
            if text.is_empty() {
                text.push_str("(no checkpoints)");
            }
            let value = serde_json::to_value(&metas)?;
            Ok(
                ToolResult::success(text, json!({ "checkpoints": value, "count": metas.len() }))
                    .with_summary(format!("{} checkpoint(s)", metas.len())),
            )
        })
    }
}

/// Restore the workspace to a checkpoint.
#[derive(Debug, Clone, Copy, Default)]
pub struct CheckpointRestoreTool;

impl Tool for CheckpointRestoreTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "checkpoint_restore",
            "Restore the whole workspace byte-exactly to a checkpoint (deletes files created since).",
            json!({
                "type": "object",
                "required": ["id"],
                "properties": { "id": { "type": "string", "minLength": 1 } },
                "additionalProperties": false
            }),
            vec![Capability::WriteFile],
            true,
        )
    }

    fn execute<'a>(
        &'a self,
        args: Json,
        ctx: &'a ToolContext,
    ) -> BoxFuture<'a, Result<ToolResult>> {
        Box::pin(async move {
            let id = CheckpointId::from_existing(require_str(&args, "id")?);
            {
                let mut store = ctx
                    .checkpoints
                    .lock()
                    .map_err(|_| CoreError::internal("checkpoint store lock poisoned"))?;
                store.restore(&id)?;
            }
            Ok(ToolResult::success(
                format!("restored workspace to {}", id.as_str()),
                json!({ "id": id.as_str() }),
            )
            .with_summary(format!("restored {}", id.as_str())))
        })
    }
}

/// Permanently delete a workspace checkpoint (the snapshot record + its
/// now-unreferenced blobs; the current workspace files are untouched).
#[derive(Debug, Clone, Copy, Default)]
pub struct CheckpointDeleteTool;

impl Tool for CheckpointDeleteTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "checkpoint_delete",
            "Delete a workspace checkpoint by id (does not change current files).",
            json!({
                "type": "object",
                "required": ["id"],
                "properties": { "id": { "type": "string", "minLength": 1 } },
                "additionalProperties": false
            }),
            vec![Capability::WriteFile],
            true,
        )
    }

    fn execute<'a>(
        &'a self,
        args: Json,
        ctx: &'a ToolContext,
    ) -> BoxFuture<'a, Result<ToolResult>> {
        Box::pin(async move {
            let id = CheckpointId::from_existing(require_str(&args, "id")?);
            {
                let mut store = ctx
                    .checkpoints
                    .lock()
                    .map_err(|_| CoreError::internal("checkpoint store lock poisoned"))?;
                store.delete(&id)?;
            }
            Ok(ToolResult::success(
                format!("deleted checkpoint {}", id.as_str()),
                json!({ "id": id.as_str(), "deleted": true }),
            )
            .with_summary(format!("deleted {}", id.as_str())))
        })
    }
}

/// Extract a required string argument or fail with `invalid_input`.
fn require_str<'a>(args: &'a Json, key: &str) -> Result<&'a str> {
    args.get(key)
        .and_then(Json::as_str)
        .ok_or_else(|| CoreError::invalid_input(format!("missing string argument {key:?}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool::ToolContextBuilder;

    fn ctx(tag: &str) -> ToolContext {
        let mut p = std::env::temp_dir();
        p.push(format!("na_tools_mem_{}_{}", tag, na_common::next_id("t")));
        ToolContextBuilder::new(p).build().unwrap()
    }

    #[tokio::test]
    async fn save_then_recall_round_trip() {
        let c = ctx("save_recall");
        let save = MemorySaveTool
            .execute(
                json!({
                    "kind": "character",
                    "title": "林惊羽",
                    "summary": "冷静的年轻剑客，主角。",
                    "content": "林惊羽出身寒门，使一柄名为‘霜寒’的长剑。",
                    "tags": ["主角", "剑客"],
                    "importance": 5
                }),
                &c,
            )
            .await
            .unwrap();
        assert!(save.ok);
        let id = save.data["id"].as_str().unwrap().to_string();

        let recall = MemoryRecallTool
            .execute(json!({ "query": "剑客", "k": 5 }), &c)
            .await
            .unwrap();
        assert!(recall.ok);
        assert!(recall.data["count"].as_u64().unwrap() >= 1);
        let hits = recall.data["hits"].as_array().unwrap();
        assert_eq!(hits[0]["id"], id);
        assert_eq!(hits[0]["title"], "林惊羽");
        // No `content` field exposed by recall.
        assert!(hits[0].get("content").is_none());
    }

    #[tokio::test]
    async fn classify_and_archive() {
        let c = ctx("classify");
        let save = MemorySaveTool
            .execute(
                json!({ "kind": "other", "title": "符文", "summary": "未知符文", "content": "刻在剑柄。" }),
                &c,
            )
            .await
            .unwrap();
        let id = save.data["id"].as_str().unwrap().to_string();

        let cl = MemoryClassifyTool
            .execute(
                json!({ "id": id, "kind": "lore", "add_tags": ["符文"] }),
                &c,
            )
            .await
            .unwrap();
        assert!(cl.ok);

        let ar = MemoryArchiveTool
            .execute(json!({ "id": id, "archived": true }), &c)
            .await
            .unwrap();
        assert!(ar.ok);
        // archived -> excluded from default recall
        let recall = MemoryRecallTool
            .execute(json!({ "query": "符文", "k": 5 }), &c)
            .await
            .unwrap();
        assert_eq!(recall.data["count"], 0);
    }

    #[tokio::test]
    async fn unknown_kind_is_invalid() {
        let c = ctx("badkind");
        let err = MemorySaveTool
            .execute(
                json!({ "kind": "nonsense", "title": "t", "summary": "s", "content": "c" }),
                &c,
            )
            .await
            .unwrap_err();
        assert!(err.is(na_common::ErrorKind::InvalidInput));
    }

    #[tokio::test]
    async fn checkpoint_create_list_restore() {
        let c = ctx("ckpt");
        // Seed a file and snapshot it.
        let abs = c.jail.resolve("note.md").unwrap();
        std::fs::write(&abs, "v1").unwrap();
        let create = CheckpointCreateTool
            .execute(json!({ "label": "initial" }), &c)
            .await
            .unwrap();
        assert!(create.ok);
        let id = create.data["id"].as_str().unwrap().to_string();

        let list = CheckpointListTool.execute(json!({}), &c).await.unwrap();
        assert_eq!(list.data["count"], 1);

        // Mutate, then restore.
        std::fs::write(&abs, "v2 changed").unwrap();
        let restore = CheckpointRestoreTool
            .execute(json!({ "id": id }), &c)
            .await
            .unwrap();
        assert!(restore.ok);
        assert_eq!(std::fs::read_to_string(&abs).unwrap(), "v1");
    }

    #[tokio::test]
    async fn classify_unknown_id_is_not_found() {
        let c = ctx("notfound");
        let err = MemoryClassifyTool
            .execute(json!({ "id": "mem_missing", "kind": "lore" }), &c)
            .await
            .unwrap_err();
        assert!(err.is(na_common::ErrorKind::NotFound));
    }

    #[tokio::test]
    async fn list_tool_returns_entries_by_kind() {
        let c = ctx("memlist");
        for (kind, title) in [
            ("character", "林惊羽"),
            ("character", "沈霜"),
            ("setting", "北境"),
        ] {
            MemorySaveTool
                .execute(
                    json!({ "kind": kind, "title": title, "summary": "概要", "content": "正文" }),
                    &c,
                )
                .await
                .unwrap();
        }

        // Filtered by kind -> only the two characters, with the structured shape
        // the UI relies on (data.entries[].{id,kind,title,summary,...}).
        let chars = MemoryListTool
            .execute(json!({ "kinds": ["character"] }), &c)
            .await
            .unwrap();
        assert!(chars.ok);
        let entries = chars.data["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().all(|e| e["kind"] == "character"));
        assert!(entries[0]["title"].is_string());
        assert!(entries[0].get("id").is_some());
        assert_eq!(chars.data["count"], 2);

        // No filter -> every entry; no `k` argument exists, so no max-50 ceiling.
        let all = MemoryListTool.execute(json!({}), &c).await.unwrap();
        assert_eq!(all.data["entries"].as_array().unwrap().len(), 3);

        // A large limit is accepted (unlike memory_recall's k<=50).
        let big = MemoryListTool
            .execute(json!({ "limit": 500 }), &c)
            .await
            .unwrap();
        assert!(big.ok);
    }
}
