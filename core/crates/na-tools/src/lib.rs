//! `na-tools` — the tool-execution layer of the Novel Generate Team core.
//!
//! This crate turns the safety primitives of `na-sandbox` and the durable stores
//! of `na-memory` into a coherent, model-facing **tool protocol**:
//!
//! * **Protocol** ([`tool`]) — the [`Tool`] trait (object-safe via [`BoxFuture`]),
//!   its [`ToolSpec`]/[`ToolResult`]/[`ResultMeta`] data types, the shared
//!   [`ToolContext`] (built with [`ToolContextBuilder`]), and the auxiliary
//!   traits [`Fetcher`], [`McpClient`] and [`Approver`] (all with mock defaults).
//!
//! * **Validation** ([`validate`]) — a dependency-light JSON-Schema
//!   [`validate`](validate::validate) used to check tool arguments before
//!   execution.
//!
//! * **Output pipeline** ([`output`]) — [`OutputProcessor`]/[`OutputLimits`],
//!   which detect binary data, strip ANSI, redact secrets, and bound output by
//!   lines and bytes; plus the error-first
//!   [`process_result`](output::OutputProcessor::process_result).
//!
//! * **Registry & lifecycle** ([`registry`]) — [`ToolRegistry`], whose
//!   [`invoke`](ToolRegistry::invoke) runs the *complete* guarded lifecycle:
//!   lookup → validate → authorize → cancel-check → execute-under-deadline →
//!   normalize-error → audit. It never panics and always returns a
//!   [`ToolResult`].
//!
//! * **Tools** ([`tools`]) — the built-in tool families (filesystem, editing,
//!   search, shell, web, MCP, a prose-oriented version store, and memory /
//!   checkpoint wrappers).
//!
//! ## Object safety
//!
//! Async traits in Rust are not `dyn`-compatible, but the runtime needs
//! `Arc<dyn Tool>`, `Arc<dyn Fetcher>`, etc. Every async trait method here
//! therefore returns a manual [`BoxFuture`] and is implemented with
//! `Box::pin(async move { ... })`, keeping the traits `Send + Sync` and
//! object-safe.
//!
//! ```no_run
//! use na_tools::{builtin_registry, ToolContextBuilder};
//! use na_common::json;
//!
//! # async fn demo() -> na_common::Result<()> {
//! let registry = builtin_registry();
//! let ctx = ToolContextBuilder::new("./workspace").build()?;
//! let result = registry
//!     .invoke("write_file", json!({ "path": "ch1.md", "content": "第一章" }), &ctx)
//!     .await;
//! assert!(result.ok);
//! # Ok(())
//! # }
//! ```

#![forbid(unsafe_code)]

pub mod hooks;
pub mod http;
pub mod mcp_stdio;
pub mod output;
pub mod registry;
pub mod tool;
pub mod tools;
pub mod validate;

// ---- Core protocol re-exports ----
pub use tool::{
    AllowAllApprover, Approver, BoxFuture, DenyAllApprover, Fetcher, McpClient, MockFetcher,
    MockMcpClient, ResultMeta, Tool, ToolConcurrency, ToolContext, ToolContextBuilder, ToolResult,
    ToolSpec,
};

pub use output::{OutputLimits, OutputProcessor, ProcessedOutput};
pub use registry::ToolRegistry;
pub use validate::validate;

// ---- Lifecycle hooks ----
pub use hooks::{DenyToolHook, HookDecision, HookRegistry, LoggingHook, ToolHook};

// ---- Real network seams (constructed & injected by the host) ----
pub use http::HttpFetcher;
pub use mcp_stdio::{InMemoryTransport, McpTransport, StdioMcpClient, StdioTransport};

// ---- Tool re-exports ----
pub use tools::edit::EditFileTool;
pub use tools::fs::{DeleteFileTool, ListDirTool, ReadFileTool, WriteFileTool};
pub use tools::git::{
    count_words, ChapterPoint, Commit, DiffLine, DiffSummary, FictionVcs, FileDiff, FileRecord,
    GitBranchTool, GitCommitTool, GitDiffTool, GitLogTool, GitRestoreTool, LineDiff, LogEntry, Tag,
};
pub use tools::mcp::McpTool;
pub use tools::memory_tools::{
    CheckpointCreateTool, CheckpointDeleteTool, CheckpointListTool, CheckpointRestoreTool,
    MemoryArchiveTool, MemoryClassifyTool, MemoryDeleteTool, MemoryListTool, MemoryRecallTool,
    MemorySaveTool,
};
pub use tools::search::SearchTool;
pub use tools::shell::ShellTool;
pub use tools::web::WebFetchTool;

// ---- Convenience common re-exports ----
pub use na_common::{CoreError, Json, Result};

use std::sync::Arc;

/// Build a [`ToolRegistry`] pre-populated with every built-in tool.
///
/// Registered tools (by name):
/// `read_file`, `write_file`, `delete_file`, `list_dir`, `edit_file`,
/// `search`, `shell`, `web_fetch`, `vcs_commit`, `vcs_log`, `vcs_diff`,
/// `vcs_restore`, `vcs_branch`, `memory_save`, `memory_recall`, `memory_list`,
/// `memory_classify`, `memory_archive`, `memory_delete`, `checkpoint_create`,
/// `checkpoint_list`, `checkpoint_restore`, `checkpoint_delete`.
///
/// MCP tools are *not* registered here because they are discovered dynamically
/// from a live [`McpClient`]; use [`McpTool::discover`] and
/// [`ToolRegistry::register`] to add them.
pub fn builtin_registry() -> ToolRegistry {
    let mut r = ToolRegistry::new();
    // `register_or_replace` is infallible and the names are all distinct, so this
    // cannot conflict; using it keeps the function panic-free.
    r.register_or_replace(Arc::new(ReadFileTool));
    r.register_or_replace(Arc::new(WriteFileTool));
    r.register_or_replace(Arc::new(DeleteFileTool));
    r.register_or_replace(Arc::new(ListDirTool));
    r.register_or_replace(Arc::new(EditFileTool));
    r.register_or_replace(Arc::new(SearchTool));
    r.register_or_replace(Arc::new(ShellTool));
    r.register_or_replace(Arc::new(WebFetchTool));
    r.register_or_replace(Arc::new(GitCommitTool));
    r.register_or_replace(Arc::new(GitLogTool));
    r.register_or_replace(Arc::new(GitDiffTool));
    r.register_or_replace(Arc::new(GitRestoreTool));
    r.register_or_replace(Arc::new(GitBranchTool));
    r.register_or_replace(Arc::new(MemorySaveTool));
    r.register_or_replace(Arc::new(MemoryRecallTool));
    r.register_or_replace(Arc::new(MemoryListTool));
    r.register_or_replace(Arc::new(MemoryClassifyTool));
    r.register_or_replace(Arc::new(MemoryArchiveTool));
    r.register_or_replace(Arc::new(MemoryDeleteTool));
    r.register_or_replace(Arc::new(CheckpointCreateTool));
    r.register_or_replace(Arc::new(CheckpointListTool));
    r.register_or_replace(Arc::new(CheckpointRestoreTool));
    r.register_or_replace(Arc::new(CheckpointDeleteTool));
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use na_common::json;

    fn temp_root(tag: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("na_tools_lib_{}_{}", tag, na_common::next_id("t")));
        p
    }

    #[test]
    fn builtin_registry_has_all_tools() {
        let r = builtin_registry();
        let names = r.names();
        for expected in [
            "read_file",
            "write_file",
            "delete_file",
            "list_dir",
            "edit_file",
            "search",
            "shell",
            "web_fetch",
            "vcs_commit",
            "vcs_log",
            "vcs_diff",
            "vcs_restore",
            "vcs_branch",
            "memory_save",
            "memory_recall",
            "memory_list",
            "memory_classify",
            "memory_archive",
            "memory_delete",
            "checkpoint_create",
            "checkpoint_list",
            "checkpoint_restore",
            "checkpoint_delete",
        ] {
            assert!(
                names.contains(&expected.to_string()),
                "missing tool {expected}"
            );
        }
        assert_eq!(r.len(), 23);
    }

    #[test]
    fn all_specs_have_valid_schemas() {
        // Every spec's input_schema must itself be a JSON object (a valid schema).
        let r = builtin_registry();
        for spec in r.list_specs() {
            assert!(
                spec.input_schema.is_object(),
                "{} schema not object",
                spec.name
            );
            assert!(!spec.name.is_empty());
            assert!(!spec.description.is_empty());
        }
    }

    #[tokio::test]
    async fn end_to_end_write_read_via_registry() {
        let r = builtin_registry();
        let ctx = ToolContextBuilder::new(temp_root("e2e")).build().unwrap();

        let w = r
            .invoke(
                "write_file",
                json!({ "path": "book/ch1.md", "content": "第一章\n林惊羽提剑而立。" }),
                &ctx,
            )
            .await;
        assert!(w.ok, "{}", w.content);

        let read = r
            .invoke("read_file", json!({ "path": "book/ch1.md" }), &ctx)
            .await;
        assert!(read.ok);
        assert!(read.content.contains("林惊羽"));
    }

    #[tokio::test]
    async fn end_to_end_memory_and_vcs() {
        let r = builtin_registry();
        let ctx = ToolContextBuilder::new(temp_root("e2e2")).build().unwrap();

        // Save a memory and recall it.
        let save = r
            .invoke(
                "memory_save",
                json!({ "kind": "character", "title": "龙王", "summary": "龙族之王", "content": "沉睡千年。", "importance": 4 }),
                &ctx,
            )
            .await;
        assert!(save.ok);
        let recall = r
            .invoke("memory_recall", json!({ "query": "龙王" }), &ctx)
            .await;
        assert!(recall.ok);
        assert!(recall.data["count"].as_u64().unwrap() >= 1);

        // Write a chapter, commit, and check the log.
        r.invoke(
            "write_file",
            json!({ "path": "ch1.md", "content": "第一章 内容" }),
            &ctx,
        )
        .await;
        let commit = r
            .invoke("vcs_commit", json!({ "message": "初稿" }), &ctx)
            .await;
        assert!(commit.ok, "{}", commit.content);
        let log = r.invoke("vcs_log", json!({}), &ctx).await;
        assert!(log.ok);
        assert_eq!(log.data["count"], 1);
    }

    #[test]
    fn traits_are_object_safe() {
        // Compile-time proof the key traits can be made into trait objects.
        fn assert_obj<T: ?Sized>() {}
        assert_obj::<dyn Tool>();
        assert_obj::<dyn Fetcher>();
        assert_obj::<dyn McpClient>();
        assert_obj::<dyn Approver>();
    }
}
