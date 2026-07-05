//! In-place file editing with three modes, tuned for editing prose.
//!
//! [`EditFileTool`] supports three `mode`s:
//!
//! * `"anchor"` — a **local** rewrite. Replace the *unique* occurrence of
//!   `old_text` with `new_text`. If `old_text` appears zero or more than once we
//!   refuse with a [`Conflict`](na_common::ErrorKind::Conflict) so an edit is
//!   never applied to the wrong spot.
//! * `"full"` — a **whole-file** rewrite. The file's content becomes `content`.
//! * `"structured"` — **structural** edits addressed by 1-based line range or by
//!   Markdown heading. The `op` is one of `replace_range`, `insert_after`,
//!   `delete_range`, `replace_section`:
//!     * `replace_range` / `delete_range`: need `start` (and `end`, defaulting to
//!       `start`) line numbers.
//!     * `insert_after`: insert `content` after `start` (use `0` to prepend).
//!     * `replace_section`: replace the body of the Markdown section whose
//!       heading text equals `section`, up to (but not including) the next
//!       heading of the same or higher level.
//!
//! All modes are gated by [`Capability::WriteFile`] and mutate the file.

use std::fs;

use na_common::{json, CoreError, Json, Result};
use na_sandbox::Capability;

use crate::tool::{BoxFuture, Tool, ToolContext, ToolResult, ToolSpec};

/// Edit a file using anchor / full / structured modes.
#[derive(Debug, Clone, Copy, Default)]
pub struct EditFileTool;

impl Tool for EditFileTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "edit_file",
            "Edit a workspace file. mode=anchor replaces a unique old_text with new_text; \
             mode=full rewrites the whole file with content; mode=structured applies a \
             structural op (replace_range/insert_after/delete_range/replace_section).",
            json!({
                "type": "object",
                "required": ["path", "mode"],
                "properties": {
                    "path": { "type": "string", "minLength": 1 },
                    "mode": { "enum": ["anchor", "full", "structured"] },
                    "old_text": { "type": "string" },
                    "new_text": { "type": "string" },
                    "content": { "type": "string" },
                    "op": { "enum": ["replace_range", "insert_after", "delete_range", "replace_section"] },
                    "start": { "type": "integer", "minimum": 0 },
                    "end": { "type": "integer", "minimum": 0 },
                    "section": { "type": "string" }
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
            let mode = require_str(&args, "mode")?;
            let abs = ctx.jail.resolve(path)?;

            let mode_owned = mode.to_string();
            let new_content = match mode {
                "anchor" => {
                    let original = read_existing(&abs, path)?;
                    let old_text = require_str(&args, "old_text")?;
                    let new_text = require_str(&args, "new_text")?;
                    apply_anchor(&original, old_text, new_text)?
                }
                "full" => require_str(&args, "content")?.to_string(),
                "structured" => {
                    let original = read_existing(&abs, path)?;
                    apply_structured(&original, &args)?
                }
                other => {
                    return Err(CoreError::invalid_input(format!(
                        "unknown edit mode {other:?}"
                    )))
                }
            };

            ctx.budget.check_bytes(new_content.len())?;
            if let Some(parent) = abs.parent() {
                fs::create_dir_all(parent).map_err(|e| {
                    CoreError::from(e).with_context(format!("creating dirs for {path}"))
                })?;
            }
            fs::write(&abs, new_content.as_bytes())
                .map_err(|e| CoreError::from(e).with_context(format!("writing {path}")))?;

            let bytes = new_content.len();
            let lines = new_content.split('\n').count();
            Ok(ToolResult::success(
                format!("edited {path} via {mode_owned} ({bytes} bytes, {lines} lines)"),
                json!({ "path": path, "mode": mode_owned, "bytes": bytes, "lines": lines }),
            )
            .with_summary(format!("edited {path}")))
        })
    }
}

/// Replace the unique occurrence of `old_text`. Errors on 0 or >1 matches.
fn apply_anchor(original: &str, old_text: &str, new_text: &str) -> Result<String> {
    if old_text.is_empty() {
        return Err(CoreError::invalid_input(
            "anchor old_text must not be empty",
        ));
    }
    let count = original.matches(old_text).count();
    match count {
        0 => Err(CoreError::conflict(
            "anchor old_text not found in file (0 matches)",
        )),
        1 => Ok(original.replacen(old_text, new_text, 1)),
        n => Err(CoreError::conflict(format!(
            "anchor old_text is not unique ({n} matches); refine it to a single location"
        ))),
    }
}

/// Apply a structured op addressed by line range or markdown heading.
fn apply_structured(original: &str, args: &Json) -> Result<String> {
    let op = require_str(args, "op")?;
    // Split into lines, preserving the ability to rejoin with '\n'. We treat the
    // file as a list of logical lines (no trailing empty line unless present).
    let had_trailing_newline = original.ends_with('\n');
    let mut lines: Vec<String> = if original.is_empty() {
        Vec::new()
    } else {
        original
            .trim_end_matches('\n')
            .split('\n')
            .map(|s| s.to_string())
            .collect()
    };

    match op {
        "replace_range" => {
            let (start, end) = range_1based(args, lines.len(), true)?;
            let content = optional_str(args, "content").unwrap_or("");
            let replacement: Vec<String> = split_content_lines(content);
            // Replace [start-1 ..= end-1].
            let s = start - 1;
            let e = end; // exclusive upper for splice
            lines.splice(s..e, replacement);
        }
        "delete_range" => {
            let (start, end) = range_1based(args, lines.len(), true)?;
            let s = start - 1;
            let e = end;
            lines.drain(s..e);
        }
        "insert_after" => {
            // 0 means prepend; N means insert after line N (1-based).
            let start = args
                .get("start")
                .and_then(Json::as_u64)
                .ok_or_else(|| CoreError::invalid_input("insert_after needs integer 'start'"))?
                as usize;
            if start > lines.len() {
                return Err(CoreError::invalid_input(format!(
                    "insert_after start {start} is past end of file ({} lines)",
                    lines.len()
                )));
            }
            let content = optional_str(args, "content").unwrap_or("");
            let insertion = split_content_lines(content);
            let at = start; // after line `start` (1-based) => index `start`
            lines.splice(at..at, insertion);
        }
        "replace_section" => {
            let section = require_str(args, "section")?;
            let content = optional_str(args, "content").unwrap_or("");
            lines = replace_markdown_section(&lines, section, content)?;
        }
        other => {
            return Err(CoreError::invalid_input(format!(
                "unknown structured op {other:?}"
            )))
        }
    }

    let mut out = lines.join("\n");
    if had_trailing_newline && !out.is_empty() {
        out.push('\n');
    }
    Ok(out)
}

/// Parse a 1-based `start`/`end` range from args, validating bounds. When
/// `require_end_default` is set, a missing `end` defaults to `start`.
fn range_1based(args: &Json, n_lines: usize, require_end_default: bool) -> Result<(usize, usize)> {
    let start = args
        .get("start")
        .and_then(Json::as_u64)
        .ok_or_else(|| CoreError::invalid_input("op needs integer 'start' (1-based)"))?
        as usize;
    let end = match args.get("end").and_then(Json::as_u64) {
        Some(e) => e as usize,
        None if require_end_default => start,
        None => return Err(CoreError::invalid_input("op needs integer 'end'")),
    };
    if start == 0 {
        return Err(CoreError::invalid_input(
            "'start' is 1-based and must be >= 1",
        ));
    }
    if end < start {
        return Err(CoreError::invalid_input(format!(
            "'end' ({end}) must be >= 'start' ({start})"
        )));
    }
    if end > n_lines {
        return Err(CoreError::conflict(format!(
            "range end {end} exceeds file length {n_lines}"
        )));
    }
    Ok((start, end))
}

/// Replace the body of the Markdown section whose heading text equals `section`.
/// The section runs from just after its heading to the next heading of the same
/// or higher level (or EOF). The heading line itself is preserved.
fn replace_markdown_section(lines: &[String], section: &str, content: &str) -> Result<Vec<String>> {
    // Find the heading line.
    let mut heading_idx = None;
    let mut heading_level = 0usize;
    for (i, line) in lines.iter().enumerate() {
        if let Some((level, text)) = parse_heading(line) {
            if text == section {
                heading_idx = Some(i);
                heading_level = level;
                break;
            }
        }
    }
    let Some(h) = heading_idx else {
        return Err(CoreError::not_found(format!(
            "markdown section heading {section:?} not found"
        )));
    };

    // Find the end of the section: next heading with level <= heading_level.
    let mut end = lines.len();
    for (i, line) in lines.iter().enumerate().skip(h + 1) {
        if let Some((level, _)) = parse_heading(line) {
            if level <= heading_level {
                end = i;
                break;
            }
        }
    }

    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    out.extend_from_slice(&lines[..=h]); // up to and including the heading
    out.extend(split_content_lines(content));
    out.extend_from_slice(&lines[end..]); // from the next heading onward
    Ok(out)
}

/// Parse an ATX Markdown heading line, returning `(level, trimmed_text)`.
fn parse_heading(line: &str) -> Option<(usize, &str)> {
    let trimmed = line.trim_start();
    if !trimmed.starts_with('#') {
        return None;
    }
    let hashes = trimmed.chars().take_while(|&c| c == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = &trimmed[hashes..];
    // A valid ATX heading needs a space after the hashes (or be just hashes).
    if !rest.is_empty() && !rest.starts_with(' ') {
        return None;
    }
    Some((hashes, rest.trim()))
}

/// Split `content` into logical lines (empty string -> no lines).
fn split_content_lines(content: &str) -> Vec<String> {
    if content.is_empty() {
        Vec::new()
    } else {
        content
            .trim_end_matches('\n')
            .split('\n')
            .map(|s| s.to_string())
            .collect()
    }
}

/// Read an existing file's text, erroring if it does not exist.
fn read_existing(abs: &std::path::Path, path: &str) -> Result<String> {
    if !abs.exists() {
        return Err(CoreError::not_found(format!("file not found: {path}")));
    }
    let bytes =
        fs::read(abs).map_err(|e| CoreError::from(e).with_context(format!("reading {path}")))?;
    String::from_utf8(bytes)
        .map_err(|_| CoreError::invalid_input(format!("file {path} is not valid UTF-8")))
}

/// Extract a required string argument or fail with `invalid_input`.
fn require_str<'a>(args: &'a Json, key: &str) -> Result<&'a str> {
    args.get(key)
        .and_then(Json::as_str)
        .ok_or_else(|| CoreError::invalid_input(format!("missing string argument {key:?}")))
}

/// Extract an optional string argument.
fn optional_str<'a>(args: &'a Json, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Json::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool::ToolContextBuilder;

    fn ctx(tag: &str) -> ToolContext {
        let mut p = std::env::temp_dir();
        p.push(format!("na_tools_edit_{}_{}", tag, na_common::next_id("t")));
        ToolContextBuilder::new(p).build().unwrap()
    }

    async fn seed(c: &ToolContext, path: &str, content: &str) {
        let abs = c.jail.resolve(path).unwrap();
        if let Some(p) = abs.parent() {
            std::fs::create_dir_all(p).unwrap();
        }
        std::fs::write(&abs, content).unwrap();
    }

    fn read(c: &ToolContext, path: &str) -> String {
        std::fs::read_to_string(c.jail.resolve(path).unwrap()).unwrap()
    }

    #[tokio::test]
    async fn anchor_unique_replaces() {
        let c = ctx("anchor_ok");
        seed(&c, "ch1.md", "他叫林惊羽，是青云门弟子。").await;
        let res = EditFileTool
            .execute(
                json!({
                    "path": "ch1.md", "mode": "anchor",
                    "old_text": "青云门", "new_text": "天剑门"
                }),
                &c,
            )
            .await
            .unwrap();
        assert!(res.ok);
        assert_eq!(read(&c, "ch1.md"), "他叫林惊羽，是天剑门弟子。");
    }

    #[tokio::test]
    async fn anchor_zero_match_conflicts() {
        let c = ctx("anchor_zero");
        seed(&c, "ch1.md", "hello world").await;
        let err = EditFileTool
            .execute(
                json!({ "path": "ch1.md", "mode": "anchor", "old_text": "missing", "new_text": "x" }),
                &c,
            )
            .await
            .unwrap_err();
        assert!(err.is(na_common::ErrorKind::Conflict));
        assert!(err.message.contains("0 matches"));
    }

    #[tokio::test]
    async fn anchor_non_unique_conflicts() {
        let c = ctx("anchor_dup");
        seed(&c, "ch1.md", "ab ab ab").await;
        let err = EditFileTool
            .execute(
                json!({ "path": "ch1.md", "mode": "anchor", "old_text": "ab", "new_text": "x" }),
                &c,
            )
            .await
            .unwrap_err();
        assert!(err.is(na_common::ErrorKind::Conflict));
        assert!(err.message.contains("not unique"));
    }

    #[tokio::test]
    async fn full_rewrites_file() {
        let c = ctx("full");
        seed(&c, "ch1.md", "old content").await;
        let res = EditFileTool
            .execute(
                json!({ "path": "ch1.md", "mode": "full", "content": "brand new\nbody" }),
                &c,
            )
            .await
            .unwrap();
        assert!(res.ok);
        assert_eq!(read(&c, "ch1.md"), "brand new\nbody");
    }

    #[tokio::test]
    async fn full_can_create_new_file() {
        let c = ctx("full_new");
        let res = EditFileTool
            .execute(
                json!({ "path": "new/file.md", "mode": "full", "content": "hi" }),
                &c,
            )
            .await
            .unwrap();
        assert!(res.ok);
        assert_eq!(read(&c, "new/file.md"), "hi");
    }

    #[tokio::test]
    async fn structured_replace_range() {
        let c = ctx("sr");
        seed(&c, "f.txt", "l1\nl2\nl3\nl4").await;
        EditFileTool
            .execute(
                json!({
                    "path": "f.txt", "mode": "structured", "op": "replace_range",
                    "start": 2, "end": 3, "content": "NEW"
                }),
                &c,
            )
            .await
            .unwrap();
        assert_eq!(read(&c, "f.txt"), "l1\nNEW\nl4");
    }

    #[tokio::test]
    async fn structured_delete_range() {
        let c = ctx("sd");
        seed(&c, "f.txt", "l1\nl2\nl3\nl4").await;
        EditFileTool
            .execute(
                json!({ "path": "f.txt", "mode": "structured", "op": "delete_range", "start": 2, "end": 3 }),
                &c,
            )
            .await
            .unwrap();
        assert_eq!(read(&c, "f.txt"), "l1\nl4");
    }

    #[tokio::test]
    async fn structured_insert_after() {
        let c = ctx("si");
        seed(&c, "f.txt", "l1\nl2").await;
        EditFileTool
            .execute(
                json!({ "path": "f.txt", "mode": "structured", "op": "insert_after", "start": 1, "content": "INS" }),
                &c,
            )
            .await
            .unwrap();
        assert_eq!(read(&c, "f.txt"), "l1\nINS\nl2");
    }

    #[tokio::test]
    async fn structured_insert_prepend_with_zero() {
        let c = ctx("si0");
        seed(&c, "f.txt", "l1\nl2").await;
        EditFileTool
            .execute(
                json!({ "path": "f.txt", "mode": "structured", "op": "insert_after", "start": 0, "content": "TOP" }),
                &c,
            )
            .await
            .unwrap();
        assert_eq!(read(&c, "f.txt"), "TOP\nl1\nl2");
    }

    #[tokio::test]
    async fn structured_replace_section() {
        let c = ctx("ss");
        let md =
            "# Title\nintro\n\n## 第一章\nold body line 1\nold body line 2\n\n## 第二章\nkeep me";
        seed(&c, "doc.md", md).await;
        EditFileTool
            .execute(
                json!({
                    "path": "doc.md", "mode": "structured", "op": "replace_section",
                    "section": "第一章", "content": "全新的内容"
                }),
                &c,
            )
            .await
            .unwrap();
        let got = read(&c, "doc.md");
        // The section body (including its trailing blank line, which belonged to
        // the section) is replaced up to the next same-level heading.
        assert!(got.contains("## 第一章\n全新的内容\n## 第二章"), "{got}");
        assert!(got.contains("keep me"));
        assert!(!got.contains("old body"));
    }

    #[tokio::test]
    async fn structured_replace_section_missing_is_not_found() {
        let c = ctx("ss_missing");
        seed(&c, "doc.md", "# Title\nbody").await;
        let err = EditFileTool
            .execute(
                json!({ "path": "doc.md", "mode": "structured", "op": "replace_section", "section": "Nope", "content": "x" }),
                &c,
            )
            .await
            .unwrap_err();
        assert!(err.is(na_common::ErrorKind::NotFound));
    }

    #[tokio::test]
    async fn structured_range_out_of_bounds_conflicts() {
        let c = ctx("oob");
        seed(&c, "f.txt", "l1\nl2").await;
        let err = EditFileTool
            .execute(
                json!({ "path": "f.txt", "mode": "structured", "op": "replace_range", "start": 1, "end": 9, "content": "x" }),
                &c,
            )
            .await
            .unwrap_err();
        assert!(err.is(na_common::ErrorKind::Conflict));
    }

    #[tokio::test]
    async fn anchor_on_missing_file_is_not_found() {
        let c = ctx("anchor_missing_file");
        let err = EditFileTool
            .execute(
                json!({ "path": "ghost.md", "mode": "anchor", "old_text": "a", "new_text": "b" }),
                &c,
            )
            .await
            .unwrap_err();
        assert!(err.is(na_common::ErrorKind::NotFound));
    }

    #[test]
    fn parse_heading_levels() {
        assert_eq!(parse_heading("## 第一章"), Some((2, "第一章")));
        assert_eq!(parse_heading("# Title "), Some((1, "Title")));
        assert_eq!(parse_heading("###### h6"), Some((6, "h6")));
        assert_eq!(parse_heading("####### too deep"), None);
        assert_eq!(parse_heading("not a heading"), None);
        assert_eq!(parse_heading("#no-space"), None);
    }
}
