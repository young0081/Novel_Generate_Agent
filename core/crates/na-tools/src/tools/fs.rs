//! Filesystem tools: read, write, and list — all confined to the [`PathJail`].
//!
//! Every path the model supplies is resolved through `ctx.jail` so a tool can
//! never touch anything outside the workspace. Read output is run through the
//! [`OutputProcessor`](crate::OutputProcessor) so binary files, secrets, and
//! oversized files are handled safely.

use std::fs;
use std::io::Write;
use std::path::Path;

use na_common::{json, CoreError, Json, Result};
use na_sandbox::Capability;

use crate::output::OutputProcessor;
use crate::tool::{BoxFuture, ResultMeta, Tool, ToolContext, ToolResult, ToolSpec};

/// Read a UTF-8 (or binary) file from the workspace.
#[derive(Debug, Clone, Copy, Default)]
pub struct ReadFileTool;

impl Tool for ReadFileTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "read_file",
            "Read the contents of a file in the workspace. Binary files are summarized; \
             large files are truncated; secrets are redacted.",
            json!({
                "type": "object",
                "required": ["path"],
                "properties": {
                    "path": { "type": "string", "minLength": 1,
                        "description": "Workspace-relative path to read." }
                },
                "additionalProperties": false
            }),
            vec![Capability::ReadFile],
            false,
        )
    }

    fn execute<'a>(
        &'a self,
        args: Json,
        ctx: &'a ToolContext,
    ) -> BoxFuture<'a, Result<ToolResult>> {
        Box::pin(async move {
            let path = require_str(&args, "path")?;
            let abs = ctx.jail.resolve(path)?;
            ensure_user_workspace_path(ctx, &abs)?;
            if !abs.exists() {
                return Err(CoreError::not_found(format!("file not found: {path}")));
            }
            let bytes = fs::read(&abs)
                .map_err(|e| CoreError::from(e).with_context(format!("reading {path}")))?;
            // Enforce the byte budget before processing.
            ctx.budget
                .check_bytes(bytes.len())
                .map_err(|e| e.with_context(format!("file {path} is {} bytes", bytes.len())))?;

            let processor = OutputProcessor::default();
            let processed = processor.process(&bytes);
            let meta = ResultMeta {
                bytes: processed.bytes,
                truncated: processed.truncated,
                was_binary: processed.was_binary,
                redactions: processed.redactions,
                untrusted: false,
                duration_ms: 0,
            };
            let data = json!({
                "path": path,
                "bytes": bytes.len(),
                "truncated": processed.truncated,
                "was_binary": processed.was_binary,
            });
            Ok(ToolResult {
                ok: true,
                content: processed.text,
                data,
                summary: Some(format!("read {path} ({} bytes)", bytes.len())),
                metadata: meta,
            })
        })
    }
}

/// Write (create or overwrite) a file in the workspace.
#[derive(Debug, Clone, Copy, Default)]
pub struct WriteFileTool;

impl Tool for WriteFileTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "write_file",
            "Create or overwrite a file in the workspace, creating parent directories as needed.",
            json!({
                "type": "object",
                "required": ["path", "content"],
                "properties": {
                    "path": { "type": "string", "minLength": 1 },
                    "content": { "type": "string" }
                },
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
            let path = require_str(&args, "path")?;
            let content = require_str(&args, "content")?;
            let abs = ctx.jail.resolve(path)?;
            ensure_user_workspace_path(ctx, &abs)?;

            // Enforce the output budget on the bytes we are about to write.
            ctx.budget.check_bytes(content.len())?;

            if let Some(parent) = abs.parent() {
                fs::create_dir_all(parent).map_err(|e| {
                    CoreError::from(e).with_context(format!("creating dirs for {path}"))
                })?;
            }
            atomic_write(&abs, content.as_bytes(), path)?;

            let bytes_written = content.len();
            Ok(ToolResult::success(
                format!("wrote {bytes_written} bytes to {path}"),
                json!({ "path": path, "bytes_written": bytes_written }),
            )
            .with_summary(format!("wrote {path}")))
        })
    }
}

/// List the entries of a directory in the workspace.
#[derive(Debug, Clone, Copy, Default)]
pub struct ListDirTool;

impl Tool for ListDirTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "list_dir",
            "List the files and subdirectories of a directory in the workspace.",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string",
                        "description": "Workspace-relative directory (default: workspace root)." }
                },
                "additionalProperties": false
            }),
            vec![Capability::ListDir],
            false,
        )
    }

    fn execute<'a>(
        &'a self,
        args: Json,
        ctx: &'a ToolContext,
    ) -> BoxFuture<'a, Result<ToolResult>> {
        Box::pin(async move {
            let path = args.get("path").and_then(Json::as_str).unwrap_or("");
            let abs = ctx.jail.resolve(path)?;
            ensure_user_workspace_path(ctx, &abs)?;
            if !abs.exists() {
                return Err(CoreError::not_found(format!("directory not found: {path}")));
            }
            if !abs.is_dir() {
                return Err(CoreError::invalid_input(format!("not a directory: {path}")));
            }

            let mut entries: Vec<Json> = Vec::new();
            let read = fs::read_dir(&abs)
                .map_err(|e| CoreError::from(e).with_context(format!("listing {path}")))?;
            for entry in read {
                let entry = entry.map_err(CoreError::from)?;
                let name = entry.file_name().to_string_lossy().into_owned();
                if abs == ctx.jail.root() && is_reserved_component(&name) {
                    continue;
                }
                let ft = entry.file_type().map_err(CoreError::from)?;
                let kind = if ft.is_dir() {
                    "dir"
                } else if ft.is_file() {
                    "file"
                } else {
                    "other"
                };
                let size = if ft.is_file() {
                    entry.metadata().map(|m| m.len()).unwrap_or(0)
                } else {
                    0
                };
                entries.push(json!({ "name": name, "kind": kind, "size": size }));
            }
            // Deterministic order: directories first, then by name.
            entries.sort_by(|a, b| {
                let ak = a["kind"].as_str().unwrap_or("");
                let bk = b["kind"].as_str().unwrap_or("");
                let an = a["name"].as_str().unwrap_or("");
                let bn = b["name"].as_str().unwrap_or("");
                (ak != "dir").cmp(&(bk != "dir")).then_with(|| an.cmp(bn))
            });

            let mut text = String::new();
            for e in &entries {
                let marker = if e["kind"] == "dir" { "/" } else { "" };
                text.push_str(e["name"].as_str().unwrap_or(""));
                text.push_str(marker);
                text.push('\n');
            }
            let count = entries.len();
            Ok(ToolResult::success(
                text,
                json!({ "path": path, "entries": entries, "count": count }),
            )
            .with_summary(format!(
                "{count} entries in {}",
                if path.is_empty() { "/" } else { path }
            )))
        })
    }
}

/// Delete a file in the workspace.
#[derive(Debug, Clone, Copy, Default)]
pub struct DeleteFileTool;

impl Tool for DeleteFileTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "delete_file",
            "Delete a single file in the workspace (refuses directories).",
            json!({
                "type": "object",
                "required": ["path"],
                "properties": { "path": { "type": "string", "minLength": 1 } },
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
            let path = require_str(&args, "path")?;
            let abs = ctx.jail.resolve(path)?;
            ensure_user_workspace_path(ctx, &abs)?;
            if !abs.exists() {
                return Err(CoreError::not_found(format!("file not found: {path}")));
            }
            if abs.is_dir() {
                return Err(CoreError::invalid_input(format!(
                    "refusing to delete a directory: {path}"
                )));
            }
            fs::remove_file(&abs)
                .map_err(|e| CoreError::from(e).with_context(format!("deleting {path}")))?;
            Ok(ToolResult::success(
                format!("deleted {path}"),
                json!({ "path": path, "deleted": true }),
            )
            .with_summary(format!("deleted {path}")))
        })
    }
}

/// Extract a required string argument or fail with `invalid_input`.
fn require_str<'a>(args: &'a Json, key: &str) -> Result<&'a str> {
    args.get(key)
        .and_then(Json::as_str)
        .ok_or_else(|| CoreError::invalid_input(format!("missing string argument {key:?}")))
}

pub(crate) fn ensure_user_workspace_path(ctx: &ToolContext, path: &Path) -> Result<()> {
    let relative = ctx.jail.relative(path).ok_or_else(|| {
        CoreError::sandbox(format!(
            "resolved path is outside workspace: {}",
            path.display()
        ))
    })?;
    if relative
        .split('/')
        .next()
        .is_some_and(is_reserved_component)
    {
        return Err(CoreError::security(format!(
            "access to internal workspace state is blocked: {relative}"
        )));
    }
    Ok(())
}

fn is_reserved_component(component: &str) -> bool {
    component.eq_ignore_ascii_case(".na")
        || component.eq_ignore_ascii_case(".na-vcs")
        || component.eq_ignore_ascii_case(".git")
}

pub(crate) fn atomic_write(path: &Path, content: &[u8], display_path: &str) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| CoreError::invalid_input("file path has no parent directory"))?;
    let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|error| {
        CoreError::from(error).with_context(format!("creating temporary file for {display_path}"))
    })?;
    temp.write_all(content).map_err(|error| {
        CoreError::from(error).with_context(format!("writing temporary file for {display_path}"))
    })?;
    temp.as_file().sync_all().map_err(|error| {
        CoreError::from(error).with_context(format!("flushing temporary file for {display_path}"))
    })?;
    temp.persist(path).map_err(|error| {
        CoreError::from(error.error).with_context(format!("replacing {display_path}"))
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool::ToolContextBuilder;
    use crate::tools::edit::EditFileTool;

    fn ctx(tag: &str) -> ToolContext {
        let mut p = std::env::temp_dir();
        p.push(format!("na_tools_fs_{}_{}", tag, na_common::next_id("t")));
        ToolContextBuilder::new(p).build().unwrap()
    }

    #[tokio::test]
    async fn write_then_read_round_trip() {
        let c = ctx("rt");
        let w = WriteFileTool;
        let res = w
            .execute(
                json!({ "path": "book/ch1.md", "content": "第一章\n林惊羽" }),
                &c,
            )
            .await
            .unwrap();
        assert!(res.ok);
        assert_eq!(res.data["bytes_written"], "第一章\n林惊羽".len());

        let r = ReadFileTool;
        let res = r
            .execute(json!({ "path": "book/ch1.md" }), &c)
            .await
            .unwrap();
        assert!(res.ok);
        assert!(res.content.contains("林惊羽"));
        assert_eq!(res.data["was_binary"], false);
    }

    #[tokio::test]
    async fn read_missing_is_not_found() {
        let c = ctx("missing");
        let r = ReadFileTool;
        let err = r
            .execute(json!({ "path": "nope.md" }), &c)
            .await
            .unwrap_err();
        assert!(err.is(na_common::ErrorKind::NotFound));
    }

    #[tokio::test]
    async fn read_rejects_escape() {
        let c = ctx("escape");
        let r = ReadFileTool;
        let err = r
            .execute(json!({ "path": "../../etc/passwd" }), &c)
            .await
            .unwrap_err();
        assert!(err.is(na_common::ErrorKind::SandboxViolation));
    }

    #[tokio::test]
    async fn read_binary_is_summarized() {
        let c = ctx("bin");
        // Write raw binary via the jail directly.
        let abs = c.jail.resolve("blob.bin").unwrap();
        std::fs::write(&abs, [0u8, 1, 2, 3, 0, 9]).unwrap();
        let r = ReadFileTool;
        let res = r.execute(json!({ "path": "blob.bin" }), &c).await.unwrap();
        assert!(res.metadata.was_binary);
        assert!(res.content.contains("[binary data:"));
    }

    #[tokio::test]
    async fn list_dir_returns_entries() {
        let c = ctx("list");
        let w = WriteFileTool;
        w.execute(json!({ "path": "a.txt", "content": "x" }), &c)
            .await
            .unwrap();
        w.execute(json!({ "path": "sub/b.txt", "content": "y" }), &c)
            .await
            .unwrap();

        let l = ListDirTool;
        let res = l.execute(json!({ "path": "" }), &c).await.unwrap();
        assert!(res.ok);
        let names: Vec<&str> = res.data["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["name"].as_str().unwrap())
            .collect();
        // Internal state is hidden from model-facing directory listings.
        assert!(names.contains(&"sub"));
        assert!(names.contains(&"a.txt"));
        assert!(!names.contains(&".na"));
    }

    #[tokio::test]
    async fn list_dir_on_file_is_invalid() {
        let c = ctx("listfile");
        WriteFileTool
            .execute(json!({ "path": "f.txt", "content": "x" }), &c)
            .await
            .unwrap();
        let err = ListDirTool
            .execute(json!({ "path": "f.txt" }), &c)
            .await
            .unwrap_err();
        assert!(err.is(na_common::ErrorKind::InvalidInput));
    }

    #[tokio::test]
    async fn write_over_budget_rejected() {
        let mut p = std::env::temp_dir();
        p.push(format!("na_tools_fs_budget_{}", na_common::next_id("t")));
        let c = ToolContextBuilder::new(p)
            .budget(na_sandbox::ResourceBudget::new(8, 1000, 50))
            .build()
            .unwrap();
        let err = WriteFileTool
            .execute(
                json!({ "path": "big.txt", "content": "way too many bytes" }),
                &c,
            )
            .await
            .unwrap_err();
        assert!(err.is(na_common::ErrorKind::BudgetExceeded));
    }

    #[test]
    fn atomic_write_replaces_existing_content_without_temp_leaks() {
        let c = ctx("atomic");
        let path = c.jail.resolve("chapter.md").unwrap();
        fs::write(&path, "old").unwrap();

        atomic_write(&path, b"new chapter", "chapter.md").unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "new chapter");
        let siblings: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(
            siblings.iter().filter(|name| *name != "chapter.md").count(),
            1
        );
        assert!(siblings.iter().any(|name| name == "chapter.md"));
    }

    #[tokio::test]
    async fn file_tools_hide_and_reject_internal_workspace_state() {
        let context = ctx("reserved");
        let root = context.jail.root();
        fs::write(root.join(".na/protected.txt"), "state secret").unwrap();
        fs::create_dir_all(root.join(".na-vcs")).unwrap();
        fs::write(root.join(".na-vcs/commits.jsonl"), "commit secret").unwrap();
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::write(root.join(".git/config"), "git secret").unwrap();

        let listing = ListDirTool
            .execute(json!({ "path": "" }), &context)
            .await
            .unwrap();
        let names: Vec<&str> = listing.data["entries"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|entry| entry["name"].as_str())
            .collect();
        assert!(!names
            .iter()
            .any(|name| matches!(*name, ".na" | ".na-vcs" | ".git")));

        let absolute = root
            .join(".na/protected.txt")
            .to_string_lossy()
            .into_owned();
        for path in [
            ".na/protected.txt".to_string(),
            "./.na/protected.txt".to_string(),
            "chapters/../.na/protected.txt".to_string(),
            ".NA/protected.txt".to_string(),
            absolute,
        ] {
            let error = ReadFileTool
                .execute(json!({ "path": path }), &context)
                .await
                .unwrap_err();
            assert!(error.is(na_common::ErrorKind::SecurityBlocked), "{error}");
        }

        for path in [
            ".na/new.json",
            ".na-vcs/state.json",
            ".git/hooks/pre-commit",
        ] {
            let error = WriteFileTool
                .execute(json!({ "path": path, "content": "blocked" }), &context)
                .await
                .unwrap_err();
            assert!(error.is(na_common::ErrorKind::SecurityBlocked), "{error}");
        }

        let edit_error = EditFileTool
            .execute(
                json!({ "path": ".na/protected.txt", "mode": "full", "content": "changed" }),
                &context,
            )
            .await
            .unwrap_err();
        assert!(edit_error.is(na_common::ErrorKind::SecurityBlocked));
        let delete_error = DeleteFileTool
            .execute(json!({ "path": ".na/protected.txt" }), &context)
            .await
            .unwrap_err();
        assert!(delete_error.is(na_common::ErrorKind::SecurityBlocked));
        let list_error = ListDirTool
            .execute(json!({ "path": ".na" }), &context)
            .await
            .unwrap_err();
        assert!(list_error.is(na_common::ErrorKind::SecurityBlocked));
        assert_eq!(
            fs::read_to_string(root.join(".na/protected.txt")).unwrap(),
            "state secret"
        );
    }
}
