//! Reusable **skills** — named, prompt-level playbooks the agent can load on
//! demand.
//!
//! A skill is a small markdown document with a YAML-ish frontmatter header
//! followed by a free-form instruction body:
//!
//! ```markdown
//! ---
//! name: outline-arc
//! description: Draft a three-act outline for a story arc.
//! allowed-tools: [write_file, memory_save]
//! ---
//! When asked to outline an arc, first recall the relevant characters, then
//! write a three-act outline to `outline.md` and save the key beats to memory.
//! ```
//!
//! Skills are deliberately *content*, not code: loading one injects its
//! instructions into the model context (see [`skill_system_message`]) so the
//! model adopts the playbook for the current run. Two tools expose the registry
//! to the agent itself:
//!
//! * [`SkillListTool`] (`skill_list`, read-only) — enumerate the available
//!   skills with their descriptions and allowed tools.
//! * [`SkillLoadTool`] (`skill_load`, read-only) — fetch one skill's full
//!   instructions by name, so the model can pull a playbook into context.
//!
//! A [`SkillRegistry`] can be built in memory ([`register`](SkillRegistry::register))
//! or loaded from a directory of `*.md` files ([`load_dir`](SkillRegistry::load_dir),
//! e.g. `.na/skills`).

use std::path::Path;
use std::sync::Arc;

use na_common::{json, CoreError, Json, Result};
use na_tools::{BoxFuture, Tool, ToolContext, ToolResult, ToolSpec};

use crate::message::Message;

/// A reusable, prompt-level playbook.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skill {
    /// Unique skill name (what the agent loads it by).
    pub name: String,
    /// One-line description for listings / selection.
    pub description: String,
    /// The instruction body injected into context when the skill is loaded.
    pub instructions: String,
    /// Tools this skill expects to use (advisory; surfaced in listings).
    pub allowed_tools: Vec<String>,
}

impl Skill {
    /// Construct a skill directly (mostly for tests / programmatic use).
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        instructions: impl Into<String>,
        allowed_tools: Vec<String>,
    ) -> Self {
        Skill {
            name: name.into(),
            description: description.into(),
            instructions: instructions.into(),
            allowed_tools,
        }
    }

    /// Parse a skill from markdown with an optional YAML-ish frontmatter block.
    ///
    /// A frontmatter block is a leading `---` line, then `key: value` lines, then
    /// a closing `---` line; the remaining text is the instruction body. The
    /// recognized keys are `name`, `description`, and `allowed-tools` (also
    /// accepted as `allowed_tools`). `allowed-tools` may be a bracketed list
    /// (`[a, b]`) or a bare comma-separated list (`a, b`).
    ///
    /// When no frontmatter is present the whole text becomes the instructions and
    /// the name defaults to empty — callers that need a name should use the
    /// `*_named` helpers or [`load_dir`](SkillRegistry::load_dir) (which derives
    /// the name from the file stem).
    ///
    /// Errors only on a frontmatter block that opens with `---` but is never
    /// closed.
    pub fn parse(text: &str) -> Result<Skill> {
        let (front, body) = split_frontmatter(text)?;

        let mut name = String::new();
        let mut description = String::new();
        let mut allowed_tools: Vec<String> = Vec::new();

        for (key, value) in front {
            match key.as_str() {
                "name" => name = unquote(&value),
                "description" => description = unquote(&value),
                "allowed-tools" | "allowed_tools" | "allowed tools" => {
                    allowed_tools = parse_tool_list(&value);
                }
                _ => {} // ignore unknown keys for forward compatibility
            }
        }

        Ok(Skill {
            name,
            description,
            instructions: body.trim().to_string(),
            allowed_tools,
        })
    }

    /// Whether `tool` is in this skill's advisory allow-list. An empty allow-list
    /// is treated as "no restriction" and returns `true` for every tool.
    pub fn allows_tool(&self, tool: &str) -> bool {
        self.allowed_tools.is_empty() || self.allowed_tools.iter().any(|t| t == tool)
    }
}

/// Split optional leading `---\n…\n---\n` frontmatter from the body.
///
/// Returns the parsed `key: value` pairs (in source order) and the remaining
/// body. When there is no leading `---`, the entire input is the body.
type Frontmatter = Vec<(String, String)>;
fn split_frontmatter(text: &str) -> Result<(Frontmatter, String)> {
    // Normalize CRLF so parsing is line-ending agnostic.
    let normalized = text.replace("\r\n", "\n");
    let trimmed_start = normalized.trim_start_matches('\u{feff}'); // strip BOM

    // Frontmatter must be the very first non-empty content. We allow leading
    // blank lines before the opening fence.
    let after_lead = trimmed_start.trim_start_matches('\n');
    if !after_lead.starts_with("---") {
        return Ok((Vec::new(), normalized));
    }
    // The opening fence is the first line; it must be exactly "---" (ignoring
    // trailing spaces) to count as frontmatter.
    let mut lines = after_lead.lines();
    let first = lines.next().unwrap_or("");
    if first.trim() != "---" {
        return Ok((Vec::new(), normalized));
    }

    let mut front: Frontmatter = Vec::new();
    let mut closed = false;
    // Track how many bytes we've consumed to recover the body slice precisely.
    for line in lines.by_ref() {
        if line.trim() == "---" {
            closed = true;
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            let key = k.trim().to_lowercase();
            let value = v.trim().to_string();
            if !key.is_empty() {
                front.push((key, value));
            }
        }
        // Lines without a colon inside frontmatter are ignored.
    }

    if !closed {
        return Err(CoreError::invalid_input(
            "skill frontmatter opened with '---' but was never closed",
        ));
    }

    // The body is everything after the closing fence line.
    let body: String = lines.collect::<Vec<_>>().join("\n");
    Ok((front, body))
}

/// Strip a single pair of matching surrounding quotes from a scalar value.
fn unquote(s: &str) -> String {
    let t = s.trim();
    if t.len() >= 2 {
        let bytes = t.as_bytes();
        let first = bytes[0];
        let last = bytes[t.len() - 1];
        if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
            return t[1..t.len() - 1].to_string();
        }
    }
    t.to_string()
}

/// Parse an `allowed-tools` value: either a bracketed list `[a, b]` or a bare
/// comma-separated list `a, b`. Quotes around individual entries are stripped.
fn parse_tool_list(value: &str) -> Vec<String> {
    let v = value.trim();
    let inner = if v.starts_with('[') && v.ends_with(']') {
        &v[1..v.len() - 1]
    } else {
        v
    };
    inner
        .split(',')
        .map(|s| unquote(s.trim()))
        .filter(|s| !s.is_empty())
        .collect()
}

/// An in-memory collection of [`Skill`]s, addressable by name.
#[derive(Debug, Clone, Default)]
pub struct SkillRegistry {
    skills: Vec<Skill>,
}

impl SkillRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        SkillRegistry { skills: Vec::new() }
    }

    /// Register a skill. A later registration with the same name replaces the
    /// earlier one (so a directory load can override a built-in default).
    pub fn register(&mut self, skill: Skill) {
        if let Some(existing) = self.skills.iter_mut().find(|s| s.name == skill.name) {
            *existing = skill;
        } else {
            self.skills.push(skill);
        }
    }

    /// Look up a skill by exact name.
    pub fn get(&self, name: &str) -> Option<&Skill> {
        self.skills.iter().find(|s| s.name == name)
    }

    /// All registered skills (in registration order).
    pub fn list(&self) -> &[Skill] {
        &self.skills
    }

    /// Number of registered skills.
    pub fn len(&self) -> usize {
        self.skills.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.skills.is_empty()
    }

    /// The names of all registered skills.
    pub fn names(&self) -> Vec<String> {
        self.skills.iter().map(|s| s.name.clone()).collect()
    }

    /// Load every `*.md` file in `dir` as a skill, registering each one.
    ///
    /// A file whose frontmatter omits `name` is named after its file stem (so
    /// `outline.md` becomes the skill `outline`). A missing directory is **not**
    /// an error — it yields zero skills — so callers can point at an optional
    /// `.na/skills` folder unconditionally. Returns the number of skills loaded.
    pub fn load_dir(&mut self, dir: impl AsRef<Path>) -> Result<usize> {
        let dir = dir.as_ref();
        if !dir.exists() {
            return Ok(0);
        }
        let entries = std::fs::read_dir(dir).map_err(|e| {
            CoreError::from(e).with_context(format!("reading skills dir {}", dir.display()))
        })?;

        // Collect-and-sort so loading is deterministic regardless of FS order.
        let mut paths: Vec<std::path::PathBuf> = Vec::new();
        for entry in entries {
            let entry = entry
                .map_err(|e| CoreError::from(e).with_context("iterating skills dir entries"))?;
            let path = entry.path();
            let is_md = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.eq_ignore_ascii_case("md"))
                .unwrap_or(false);
            if path.is_file() && is_md {
                paths.push(path);
            }
        }
        paths.sort();

        let mut loaded = 0usize;
        for path in paths {
            let text = std::fs::read_to_string(&path).map_err(|e| {
                CoreError::from(e).with_context(format!("reading skill file {}", path.display()))
            })?;
            let mut skill = Skill::parse(&text)?;
            if skill.name.trim().is_empty() {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    skill.name = stem.to_string();
                }
            }
            // Skip anonymous skills that still have no usable name.
            if skill.name.trim().is_empty() {
                continue;
            }
            self.register(skill);
            loaded += 1;
        }
        Ok(loaded)
    }
}

/// Build a [`System`](Message::system) message that injects a skill's
/// instructions into the model context.
pub fn skill_system_message(skill: &Skill) -> Message {
    let mut body = format!("# Skill: {}\n", skill.name);
    if !skill.description.is_empty() {
        body.push_str(&format!("{}\n", skill.description));
    }
    if !skill.allowed_tools.is_empty() {
        body.push_str(&format!(
            "Allowed tools: {}\n",
            skill.allowed_tools.join(", ")
        ));
    }
    body.push('\n');
    body.push_str(&skill.instructions);
    Message::system(body)
}

/// JSON header describing a skill for tool output (no full instructions).
fn skill_header(skill: &Skill) -> Json {
    json!({
        "name": skill.name,
        "description": skill.description,
        "allowed_tools": skill.allowed_tools,
    })
}

/// A read-only tool that lists the available skills.
#[derive(Clone)]
pub struct SkillListTool {
    registry: Arc<SkillRegistry>,
}

impl std::fmt::Debug for SkillListTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SkillListTool")
            .field("skills", &self.registry.len())
            .finish()
    }
}

impl SkillListTool {
    /// Build the tool backed by `registry`.
    pub fn new(registry: Arc<SkillRegistry>) -> Self {
        SkillListTool { registry }
    }
}

impl Tool for SkillListTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "skill_list",
            "List the available reusable skills (playbooks) with their \
             descriptions and allowed tools. Read-only.",
            json!({ "type": "object", "additionalProperties": false }),
            vec![],
            false,
        )
    }

    fn execute<'a>(
        &'a self,
        _args: Json,
        _ctx: &'a ToolContext,
    ) -> BoxFuture<'a, Result<ToolResult>> {
        Box::pin(async move {
            let headers: Vec<Json> = self.registry.list().iter().map(skill_header).collect();
            let names = self.registry.names();
            let content = if names.is_empty() {
                "No skills available.".to_string()
            } else {
                format!("{} skill(s): {}", names.len(), names.join(", "))
            };
            Ok(ToolResult::success(
                content,
                json!({ "count": headers.len(), "skills": headers }),
            ))
        })
    }
}

/// A read-only tool that returns one skill's full instructions by name.
#[derive(Clone)]
pub struct SkillLoadTool {
    registry: Arc<SkillRegistry>,
}

impl std::fmt::Debug for SkillLoadTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SkillLoadTool")
            .field("skills", &self.registry.len())
            .finish()
    }
}

impl SkillLoadTool {
    /// Build the tool backed by `registry`.
    pub fn new(registry: Arc<SkillRegistry>) -> Self {
        SkillLoadTool { registry }
    }
}

impl Tool for SkillLoadTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "skill_load",
            "Load a reusable skill by name, returning its full instructions so \
             you can follow that playbook. Read-only.",
            json!({
                "type": "object",
                "required": ["name"],
                "properties": { "name": { "type": "string", "minLength": 1 } },
                "additionalProperties": false
            }),
            vec![],
            false,
        )
    }

    fn execute<'a>(
        &'a self,
        args: Json,
        _ctx: &'a ToolContext,
    ) -> BoxFuture<'a, Result<ToolResult>> {
        Box::pin(async move {
            let name = args
                .get("name")
                .and_then(Json::as_str)
                .ok_or_else(|| CoreError::invalid_input("missing string argument \"name\""))?;
            match self.registry.get(name) {
                Some(skill) => Ok(ToolResult::success(
                    skill.instructions.clone(),
                    json!({
                        "name": skill.name,
                        "description": skill.description,
                        "allowed_tools": skill.allowed_tools,
                        "instructions": skill.instructions,
                    }),
                )
                .with_summary(format!("loaded skill {}", skill.name))),
                None => Err(CoreError::not_found(format!("unknown skill {name:?}"))),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use na_tools::ToolContextBuilder;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "na_runtime_skills_{}_{}",
            tag,
            na_common::next_id("t")
        ));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn parse_frontmatter_with_bracketed_tool_list() {
        let text = "---\nname: outline-arc\ndescription: Draft an arc outline.\nallowed-tools: [write_file, memory_save]\n---\nWrite a three-act outline.\nThen save the beats.\n";
        let skill = Skill::parse(text).unwrap();
        assert_eq!(skill.name, "outline-arc");
        assert_eq!(skill.description, "Draft an arc outline.");
        assert_eq!(
            skill.allowed_tools,
            vec!["write_file".to_string(), "memory_save".to_string()]
        );
        assert!(skill.instructions.starts_with("Write a three-act outline."));
        assert!(skill.instructions.contains("save the beats"));
    }

    #[test]
    fn parse_frontmatter_with_comma_tool_list_and_quotes() {
        let text = "---\nname: \"voice\"\ndescription: 'Author voice.'\nallowed_tools: write_file, \"edit_file\"\n---\nBody here.\n";
        let skill = Skill::parse(text).unwrap();
        assert_eq!(skill.name, "voice");
        assert_eq!(skill.description, "Author voice.");
        assert_eq!(
            skill.allowed_tools,
            vec!["write_file".to_string(), "edit_file".to_string()]
        );
        assert_eq!(skill.instructions, "Body here.");
    }

    #[test]
    fn parse_without_frontmatter_is_all_body() {
        let skill = Skill::parse("just instructions, no header").unwrap();
        assert_eq!(skill.name, "");
        assert_eq!(skill.instructions, "just instructions, no header");
        assert!(skill.allowed_tools.is_empty());
    }

    #[test]
    fn parse_crlf_frontmatter() {
        let text = "---\r\nname: win\r\ndescription: crlf\r\n---\r\nBody on windows.\r\n";
        let skill = Skill::parse(text).unwrap();
        assert_eq!(skill.name, "win");
        assert_eq!(skill.description, "crlf");
        assert_eq!(skill.instructions, "Body on windows.");
    }

    #[test]
    fn parse_unclosed_frontmatter_errors() {
        let text = "---\nname: broken\ndescription: no closing fence\nstill going\n";
        let err = Skill::parse(text).unwrap_err();
        assert!(err.is(na_common::ErrorKind::InvalidInput));
    }

    #[test]
    fn allows_tool_logic() {
        let restricted = Skill::new("a", "", "x", vec!["write_file".to_string()]);
        assert!(restricted.allows_tool("write_file"));
        assert!(!restricted.allows_tool("shell"));
        let open = Skill::new("b", "", "x", vec![]);
        assert!(open.allows_tool("anything"));
    }

    #[test]
    fn registry_register_get_list_and_replace() {
        let mut reg = SkillRegistry::new();
        assert!(reg.is_empty());
        reg.register(Skill::new("s1", "d1", "i1", vec![]));
        reg.register(Skill::new("s2", "d2", "i2", vec![]));
        assert_eq!(reg.len(), 2);
        assert_eq!(reg.get("s1").unwrap().description, "d1");
        // Replacement keeps the count stable and updates the content.
        reg.register(Skill::new("s1", "d1b", "i1b", vec![]));
        assert_eq!(reg.len(), 2);
        assert_eq!(reg.get("s1").unwrap().description, "d1b");
        assert_eq!(reg.names(), vec!["s1".to_string(), "s2".to_string()]);
    }

    #[test]
    fn load_dir_reads_md_files_and_derives_names() {
        let dir = temp_dir("loaddir");
        std::fs::write(
            dir.join("with_name.md"),
            "---\nname: explicit\ndescription: has a name\n---\nDo the thing.",
        )
        .unwrap();
        // No name in frontmatter => derived from the file stem "stemmed".
        std::fs::write(
            dir.join("stemmed.md"),
            "---\ndescription: name from stem\n---\nStem body.",
        )
        .unwrap();
        // A non-md file must be ignored.
        std::fs::write(dir.join("notes.txt"), "ignore me").unwrap();

        let mut reg = SkillRegistry::new();
        let n = reg.load_dir(&dir).unwrap();
        assert_eq!(n, 2);
        assert_eq!(reg.len(), 2);
        assert!(reg.get("explicit").is_some());
        let stemmed = reg.get("stemmed").unwrap();
        assert_eq!(stemmed.description, "name from stem");
        assert_eq!(stemmed.instructions, "Stem body.");
    }

    #[test]
    fn load_dir_missing_is_zero_not_error() {
        let mut reg = SkillRegistry::new();
        let mut p = std::env::temp_dir();
        p.push(format!(
            "na_runtime_skills_missing_{}",
            na_common::next_id("t")
        ));
        let n = reg.load_dir(&p).unwrap();
        assert_eq!(n, 0);
        assert!(reg.is_empty());
    }

    #[test]
    fn skill_system_message_carries_instructions() {
        let skill = Skill::new(
            "outline",
            "Outline a story.",
            "Write three acts.",
            vec!["write_file".to_string()],
        );
        let msg = skill_system_message(&skill);
        assert!(msg.is_system());
        assert!(msg.content.contains("Skill: outline"));
        assert!(msg.content.contains("Outline a story."));
        assert!(msg.content.contains("write_file"));
        assert!(msg.content.contains("Write three acts."));
    }

    #[tokio::test]
    async fn skill_list_tool_returns_headers() {
        let mut reg = SkillRegistry::new();
        reg.register(Skill::new(
            "a",
            "first",
            "ia",
            vec!["write_file".to_string()],
        ));
        reg.register(Skill::new("b", "second", "ib", vec![]));
        let reg = Arc::new(reg);
        let tool = SkillListTool::new(reg);

        let ctx = ToolContextBuilder::new(temp_dir("listtool"))
            .build()
            .unwrap();
        let res = tool.execute(json!({}), &ctx).await.unwrap();
        assert!(res.ok);
        assert_eq!(res.data["count"], 2);
        assert_eq!(res.data["skills"][0]["name"], "a");
        assert_eq!(res.data["skills"][0]["description"], "first");
        assert_eq!(res.data["skills"][0]["allowed_tools"][0], "write_file");
        assert!(res.content.contains("a"));
        assert!(res.content.contains("b"));
    }

    #[tokio::test]
    async fn skill_load_tool_returns_instructions_or_not_found() {
        let mut reg = SkillRegistry::new();
        reg.register(Skill::new(
            "outline",
            "Outline.",
            "Write three acts.",
            vec![],
        ));
        let reg = Arc::new(reg);
        let tool = SkillLoadTool::new(reg);
        let ctx = ToolContextBuilder::new(temp_dir("loadtool"))
            .build()
            .unwrap();

        let ok = tool
            .execute(json!({ "name": "outline" }), &ctx)
            .await
            .unwrap();
        assert!(ok.ok);
        assert_eq!(ok.content, "Write three acts.");
        assert_eq!(ok.data["instructions"], "Write three acts.");

        let miss = tool.execute(json!({ "name": "nope" }), &ctx).await;
        assert!(miss.is_err());
        assert!(miss.unwrap_err().is(na_common::ErrorKind::NotFound));
    }

    #[tokio::test]
    async fn skill_load_tool_via_registry_lifecycle() {
        // Exercise the full guarded lifecycle through ToolRegistry::invoke so the
        // schema validation path is covered too.
        let mut sreg = SkillRegistry::new();
        sreg.register(Skill::new("voice", "Author voice.", "Be terse.", vec![]));
        let sreg = Arc::new(sreg);

        let mut reg = na_tools::ToolRegistry::new();
        reg.register(Arc::new(SkillLoadTool::new(sreg.clone())))
            .unwrap();
        reg.register(Arc::new(SkillListTool::new(sreg))).unwrap();

        let ctx = ToolContextBuilder::new(temp_dir("lifecycle"))
            .build()
            .unwrap();
        let res = reg
            .invoke("skill_load", json!({ "name": "voice" }), &ctx)
            .await;
        assert!(res.ok);
        assert_eq!(res.content, "Be terse.");

        // Missing required arg => invalid_input via the schema validator.
        let bad = reg.invoke("skill_load", json!({}), &ctx).await;
        assert!(!bad.ok);
        assert_eq!(bad.data["code"], "invalid_input");

        let list = reg.invoke("skill_list", json!({}), &ctx).await;
        assert!(list.ok);
        assert_eq!(list.data["count"], 1);
    }
}
