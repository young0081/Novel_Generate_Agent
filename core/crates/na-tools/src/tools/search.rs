//! Workspace search: match files by name glob and/or contents by regex.
//!
//! [`SearchTool`] walks the workspace (confined to the [`PathJail`]), skipping
//! the internal `.na` state dir and any `.git` dir. A file qualifies when its
//! workspace-relative path matches `name_glob` (if given) AND, when
//! `content_regex` is given, at least one line matches the regex. Each content
//! match is returned as a `{file, line, snippet}` record; a name-only match is
//! returned with `line = 0`.

use std::fs;
use std::path::Path;

use na_common::{json, CoreError, Json, Result};
use na_sandbox::{glob_match, Capability};
use regex::Regex;

use crate::output::OutputProcessor;
use crate::tool::{BoxFuture, Tool, ToolContext, ToolResult, ToolSpec};

/// Default cap on the number of returned matches.
const DEFAULT_MAX_RESULTS: usize = 100;

/// Search the workspace by filename glob and/or content regex.
#[derive(Debug, Clone, Copy, Default)]
pub struct SearchTool;

impl Tool for SearchTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "search",
            "Search the workspace. Provide name_glob to match file paths and/or content_regex \
             to match file contents. Returns {file, line, snippet} matches.",
            json!({
                "type": "object",
                "properties": {
                    "name_glob": { "type": "string",
                        "description": "Glob over workspace-relative paths, e.g. 'book/**/*.md'." },
                    "content_regex": { "type": "string",
                        "description": "Regex matched per-line against file contents." },
                    "max_results": { "type": "integer", "minimum": 1, "maximum": 10000 }
                },
                "additionalProperties": false
            }),
            vec![Capability::ListDir, Capability::ReadFile],
            false,
        )
    }

    fn execute<'a>(
        &'a self,
        args: Json,
        ctx: &'a ToolContext,
    ) -> BoxFuture<'a, Result<ToolResult>> {
        Box::pin(async move {
            let name_glob = args.get("name_glob").and_then(Json::as_str);
            let content_regex = args.get("content_regex").and_then(Json::as_str);
            let max_results = args
                .get("max_results")
                .and_then(Json::as_u64)
                .map(|n| n as usize)
                .unwrap_or(DEFAULT_MAX_RESULTS);

            if name_glob.is_none() && content_regex.is_none() {
                return Err(CoreError::invalid_input(
                    "search needs at least one of 'name_glob' or 'content_regex'",
                ));
            }

            let regex = match content_regex {
                Some(p) => Some(Regex::new(p).map_err(|e| {
                    CoreError::invalid_input(format!("invalid content_regex {p:?}: {e}"))
                })?),
                None => None,
            };

            let root = ctx.jail.root().to_path_buf();
            let mut matches: Vec<Json> = Vec::new();
            walk(
                ctx,
                &root,
                name_glob,
                regex.as_ref(),
                max_results,
                &mut matches,
            )?;

            // Stable order: by file then line.
            matches.sort_by(|a, b| {
                let af = a["file"].as_str().unwrap_or("");
                let bf = b["file"].as_str().unwrap_or("");
                let al = a["line"].as_u64().unwrap_or(0);
                let bl = b["line"].as_u64().unwrap_or(0);
                af.cmp(bf).then(al.cmp(&bl))
            });

            // Render a compact textual view through the output pipeline.
            let mut text = String::new();
            for m in &matches {
                let file = m["file"].as_str().unwrap_or("");
                let line = m["line"].as_u64().unwrap_or(0);
                let snippet = m["snippet"].as_str().unwrap_or("");
                if line > 0 {
                    text.push_str(&format!("{file}:{line}: {snippet}\n"));
                } else {
                    text.push_str(&format!("{file}\n"));
                }
            }
            let processed = OutputProcessor::default().process(text.as_bytes());
            let count = matches.len();
            Ok(ToolResult::success(
                processed.text,
                json!({ "matches": matches, "count": count }),
            )
            .with_summary(format!("{count} match(es)")))
        })
    }
}

/// Recursively walk `dir`, collecting matches up to `max_results`.
fn walk(
    ctx: &ToolContext,
    dir: &Path,
    name_glob: Option<&str>,
    regex: Option<&Regex>,
    max_results: usize,
    out: &mut Vec<Json>,
) -> Result<()> {
    if out.len() >= max_results {
        return Ok(());
    }
    let read = match fs::read_dir(dir) {
        Ok(r) => r,
        Err(_) => return Ok(()), // unreadable dir: skip rather than abort
    };
    // Collect & sort entries for deterministic traversal.
    let mut entries: Vec<_> = read.flatten().collect();
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        if out.len() >= max_results {
            break;
        }
        let path = entry.path();
        if is_ignored(&path) {
            continue;
        }
        let ft = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };
        if ft.is_dir() {
            walk(ctx, &path, name_glob, regex, max_results, out)?;
        } else if ft.is_file() {
            let rel = match ctx.jail.relative(&path) {
                Some(r) => r,
                None => continue,
            };
            let name_ok = match name_glob {
                Some(g) => glob_match(g, &rel),
                None => true,
            };
            if !name_ok {
                continue;
            }

            match regex {
                None => {
                    // Name-only match.
                    out.push(json!({ "file": rel, "line": 0, "snippet": "" }));
                }
                Some(re) => {
                    // Read and scan lines. Skip files that are too big or binary.
                    let Ok(bytes) = fs::read(&path) else { continue };
                    if ctx.budget.check_bytes(bytes.len()).is_err() {
                        continue;
                    }
                    let Ok(text) = String::from_utf8(bytes) else {
                        continue; // skip binary / non-UTF8
                    };
                    for (i, line) in text.split('\n').enumerate() {
                        if out.len() >= max_results {
                            break;
                        }
                        if re.is_match(line) {
                            let snippet = trim_snippet(line);
                            out.push(json!({
                                "file": rel,
                                "line": (i + 1) as u64,
                                "snippet": snippet
                            }));
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

/// Skip the internal `.na` state dir and `.git`.
fn is_ignored(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|n| n.to_str()),
        Some(".na") | Some(".git")
    )
}

/// Trim a matched line to a reasonable snippet length.
fn trim_snippet(line: &str) -> String {
    const MAX: usize = 200;
    let trimmed = line.trim_end();
    if trimmed.chars().count() <= MAX {
        trimmed.to_string()
    } else {
        let cut: String = trimmed.chars().take(MAX).collect();
        format!("{cut}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool::ToolContextBuilder;

    fn ctx(tag: &str) -> ToolContext {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "na_tools_search_{}_{}",
            tag,
            na_common::next_id("t")
        ));
        ToolContextBuilder::new(p).build().unwrap()
    }

    fn seed(c: &ToolContext, path: &str, content: &str) {
        let abs = c.jail.resolve(path).unwrap();
        if let Some(p) = abs.parent() {
            std::fs::create_dir_all(p).unwrap();
        }
        std::fs::write(&abs, content).unwrap();
    }

    #[tokio::test]
    async fn name_glob_matches_files() {
        let c = ctx("glob");
        seed(&c, "book/ch1.md", "x");
        seed(&c, "book/ch2.md", "y");
        seed(&c, "notes.txt", "z");
        let res = SearchTool
            .execute(json!({ "name_glob": "book/**/*.md" }), &c)
            .await
            .unwrap();
        let files: Vec<&str> = res.data["matches"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["file"].as_str().unwrap())
            .collect();
        assert_eq!(files, vec!["book/ch1.md", "book/ch2.md"]);
    }

    #[tokio::test]
    async fn content_regex_matches_lines() {
        let c = ctx("regex");
        seed(&c, "a.md", "first line\nhas 林惊羽 here\nlast");
        seed(&c, "b.md", "nothing relevant");
        let res = SearchTool
            .execute(json!({ "content_regex": "林惊羽" }), &c)
            .await
            .unwrap();
        let matches = res.data["matches"].as_array().unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0]["file"], "a.md");
        assert_eq!(matches[0]["line"], 2);
        assert!(matches[0]["snippet"].as_str().unwrap().contains("林惊羽"));
    }

    #[tokio::test]
    async fn combined_glob_and_regex() {
        let c = ctx("combo");
        seed(&c, "src/a.rs", "fn main() {}");
        seed(&c, "src/b.txt", "fn main() {}"); // wrong extension
        let res = SearchTool
            .execute(
                json!({ "name_glob": "**/*.rs", "content_regex": "fn\\s+main" }),
                &c,
            )
            .await
            .unwrap();
        let matches = res.data["matches"].as_array().unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0]["file"], "src/a.rs");
    }

    #[tokio::test]
    async fn max_results_caps_output() {
        let c = ctx("cap");
        for i in 0..20 {
            seed(&c, &format!("f{i}.txt"), "match");
        }
        let res = SearchTool
            .execute(json!({ "content_regex": "match", "max_results": 5 }), &c)
            .await
            .unwrap();
        assert_eq!(res.data["count"], 5);
    }

    #[tokio::test]
    async fn missing_both_args_is_invalid() {
        let c = ctx("none");
        let err = SearchTool.execute(json!({}), &c).await.unwrap_err();
        assert!(err.is(na_common::ErrorKind::InvalidInput));
    }

    #[tokio::test]
    async fn invalid_regex_is_invalid_input() {
        let c = ctx("badre");
        let err = SearchTool
            .execute(json!({ "content_regex": "[" }), &c)
            .await
            .unwrap_err();
        assert!(err.is(na_common::ErrorKind::InvalidInput));
    }

    #[tokio::test]
    async fn na_state_dir_is_skipped() {
        let c = ctx("skipna");
        seed(&c, "real.md", "needle");
        // The .na dir exists (audit/memory). Ensure search does not match inside it.
        let res = SearchTool
            .execute(json!({ "content_regex": "needle" }), &c)
            .await
            .unwrap();
        let files: Vec<&str> = res.data["matches"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["file"].as_str().unwrap())
            .collect();
        assert!(files.iter().all(|f| !f.starts_with(".na")));
        assert!(files.contains(&"real.md"));
    }
}
