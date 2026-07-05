//! Core tool protocol: the [`Tool`] trait, its specification and result types,
//! and the [`ToolContext`] that carries every capability a tool needs.
//!
//! The agent's only window onto the outside world is a set of tools. Each tool
//! advertises a JSON-Schema [`ToolSpec`], declares the [`Capability`] gates it
//! must pass, and produces a structured [`ToolResult`] whose textual `content`
//! has already been run through the output-processing pipeline (ANSI stripped,
//! secrets redacted, output truncated).
//!
//! ## Object safety
//!
//! Rust supports `async fn` in traits, but such a trait is **not**
//! `dyn`-compatible. Because the runtime stores tools as `Arc<dyn Tool>` and
//! plumbs fetchers / MCP clients / approvers as trait objects, every async trait
//! here returns a manual boxed future ([`BoxFuture`]) instead of using
//! `async fn`. All these traits are `Send + Sync` so they can cross task
//! boundaries.

use std::sync::Arc;

use na_common::time::now_millis;
use na_common::CancellationToken;
use na_common::{CoreError, Json, Result, SessionId};
use na_sandbox::{Capability, CommandPolicy, Decision, PathJail, PermissionPolicy, ResourceBudget};
use serde::{Deserialize, Serialize};

use na_memory::{AuditLog, CheckpointStore, MemoryStore};

use crate::hooks::HookRegistry;

/// A boxed, `Send` future with an explicit lifetime — the object-safe stand-in
/// for `async fn` in the traits below.
pub type BoxFuture<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

/// How a tool may run relative to other calls in the same model turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolConcurrency {
    /// Safe to run with other read-only calls.
    #[default]
    ReadOnly,
    /// Delegated child-agent work; may run in parallel with other subagents but
    /// should not race with normal workspace writes.
    Subagent,
    /// Must run serially in request order.
    Mutating,
}

/// The static description of a tool: its name, human/model-readable description,
/// JSON-Schema for its arguments, the capabilities it exercises, and whether it
/// mutates state (so the runtime can decide to checkpoint first).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSpec {
    /// Unique tool name (what the model calls).
    pub name: String,
    /// What the tool does, for the model and the UI.
    pub description: String,
    /// JSON Schema describing the `args` object the tool accepts.
    pub input_schema: Json,
    /// Capabilities that must be authorized before the tool runs.
    pub capabilities: Vec<Capability>,
    /// `true` if invoking the tool can change persistent state.
    pub mutating: bool,
    /// Scheduler hint for safe batching. Defaults from `mutating` when omitted.
    #[serde(default)]
    pub concurrency: ToolConcurrency,
}

impl ToolSpec {
    /// Convenience constructor.
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        input_schema: Json,
        capabilities: Vec<Capability>,
        mutating: bool,
    ) -> Self {
        ToolSpec {
            name: name.into(),
            description: description.into(),
            input_schema,
            capabilities,
            mutating,
            concurrency: if mutating {
                ToolConcurrency::Mutating
            } else {
                ToolConcurrency::ReadOnly
            },
        }
    }

    /// Mark this tool as a subagent-style delegated task.
    pub fn with_subagent_concurrency(mut self) -> Self {
        self.concurrency = ToolConcurrency::Subagent;
        self
    }
}

/// Metadata describing how a tool's output was processed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ResultMeta {
    /// Number of bytes in the (post-processing) textual content.
    pub bytes: usize,
    /// Whether the output was truncated to fit a limit.
    pub truncated: bool,
    /// Whether the raw output was detected as binary.
    pub was_binary: bool,
    /// How many secrets were redacted from the output.
    pub redactions: u32,
    /// `true` when the content originated outside the workspace (web/MCP) and
    /// must be treated as untrusted by the prompt-injection guard.
    pub untrusted: bool,
    /// Wall-clock duration of the tool execution in milliseconds.
    pub duration_ms: u64,
}

/// The outcome of a tool invocation.
///
/// `content` is the processed text the model should read; `data` is a structured
/// payload (paths, byte counts, match lists, ...) for programmatic consumers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolResult {
    /// Whether the tool succeeded.
    pub ok: bool,
    /// Processed text intended for the model.
    pub content: String,
    /// Structured machine-readable result.
    pub data: Json,
    /// Optional short one-line summary for UIs / logs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Output-processing metadata.
    pub metadata: ResultMeta,
}

impl ToolResult {
    /// A successful result with the given processed content and structured data.
    pub fn success(content: impl Into<String>, data: Json) -> Self {
        let content = content.into();
        let bytes = content.len();
        ToolResult {
            ok: true,
            content,
            data,
            summary: None,
            metadata: ResultMeta {
                bytes,
                ..ResultMeta::default()
            },
        }
    }

    /// An error result. The content is error-first: `"[error:<code>] <message>"`.
    /// The structured `data` carries the serialized [`CoreError`].
    pub fn from_error(err: &CoreError) -> Self {
        let content = format!("[error:{}] {}", err.code, err.message);
        let bytes = content.len();
        let data = serde_json::to_value(err).unwrap_or(Json::Null);
        ToolResult {
            ok: false,
            content,
            data,
            summary: Some(format!("error: {}", err.code)),
            metadata: ResultMeta {
                bytes,
                ..ResultMeta::default()
            },
        }
    }

    /// Attach a summary (builder style).
    pub fn with_summary(mut self, summary: impl Into<String>) -> Self {
        self.summary = Some(summary.into());
        self
    }

    /// Overwrite the metadata block (builder style).
    pub fn with_metadata(mut self, metadata: ResultMeta) -> Self {
        self.metadata = metadata;
        self
    }
}

/// Decides whether an [`Ask`](Decision::Ask) permission may proceed.
///
/// Object-safe and `Send + Sync` so the runtime can swap in an interactive
/// approver (one that prompts the user) behind a trait object. The synchronous
/// `bool` return keeps callers simple; an async approver can block on its own
/// channel internally.
pub trait Approver: Send + Sync {
    /// Return `true` to allow the `(capability, resource)` action.
    fn approve(&self, capability: Capability, resource: &str) -> bool;
}

/// An approver that authorizes every `Ask` (useful as a default / in tests).
#[derive(Debug, Clone, Copy, Default)]
pub struct AllowAllApprover;

impl Approver for AllowAllApprover {
    fn approve(&self, _capability: Capability, _resource: &str) -> bool {
        true
    }
}

/// An approver that rejects every `Ask`.
#[derive(Debug, Clone, Copy, Default)]
pub struct DenyAllApprover;

impl Approver for DenyAllApprover {
    fn approve(&self, _capability: Capability, _resource: &str) -> bool {
        false
    }
}

/// Everything a tool needs to do its job, bundled into one shareable context.
///
/// The heavy stateful stores ([`MemoryStore`], [`CheckpointStore`],
/// [`AuditLog`]) live behind `Arc<Mutex<…>>` so the context is cheap to clone
/// and safe to share across tasks. Build one with [`ToolContextBuilder`].
#[derive(Clone)]
pub struct ToolContext {
    /// Filesystem boundary; every path is resolved through this.
    pub jail: PathJail,
    /// Capability permission policy.
    pub policy: PermissionPolicy,
    /// Shell-command safety policy.
    pub command_policy: CommandPolicy,
    /// Resource ceilings for a single tool call.
    pub budget: ResourceBudget,
    /// Cooperative cancellation signal.
    pub cancel: CancellationToken,
    /// Append-only audit log.
    pub audit: Arc<std::sync::Mutex<AuditLog>>,
    /// Long-term memory store.
    pub memory: Arc<std::sync::Mutex<MemoryStore>>,
    /// Workspace snapshot store.
    pub checkpoints: Arc<std::sync::Mutex<CheckpointStore>>,
    /// Outbound HTTP fetcher (mockable).
    pub fetcher: Arc<dyn Fetcher>,
    /// MCP client for remote tools (mockable).
    pub mcp: Arc<dyn McpClient>,
    /// Human approver consulted on `Ask` decisions.
    pub approver: Arc<dyn Approver>,
    /// Tool-lifecycle hooks run around every invocation (default: empty).
    pub hooks: Arc<HookRegistry>,
    /// The owning session id (recorded in audit entries).
    pub session: SessionId,
}

impl std::fmt::Debug for ToolContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolContext")
            .field("jail_root", &self.jail.root())
            .field("policy", &self.policy)
            .field("command_policy", &self.command_policy)
            .field("budget", &self.budget)
            .field("hooks", &self.hooks)
            .field("session", &self.session)
            .finish_non_exhaustive()
    }
}

impl ToolContext {
    /// Start building a context rooted at `workspace_root`.
    pub fn builder(workspace_root: impl AsRef<std::path::Path>) -> ToolContextBuilder {
        ToolContextBuilder::new(workspace_root)
    }

    /// Consult the permission policy for `(capability, resource)`.
    ///
    /// * [`Decision::Allow`] -> `Ok(())`.
    /// * [`Decision::Deny`] -> `Err(permission_denied)`.
    /// * [`Decision::Ask`] -> defer to the [`Approver`]; allow only if it
    ///   returns `true`, otherwise `Err(permission_denied)`.
    pub fn require(&self, capability: Capability, resource: &str) -> Result<()> {
        match self.policy.evaluate(capability, resource) {
            Decision::Allow => Ok(()),
            Decision::Deny => Err(CoreError::permission_denied(format!(
                "policy denies {capability:?} on {resource}"
            ))),
            Decision::Ask => {
                if self.approver.approve(capability, resource) {
                    Ok(())
                } else {
                    Err(CoreError::permission_denied(format!(
                        "approval denied for {capability:?} on {resource}"
                    )))
                }
            }
        }
    }

    /// Record an audit entry, swallowing a poisoned lock into an internal error.
    pub(crate) fn audit_record(&self, entry: na_memory::AuditEntry) {
        if let Ok(log) = self.audit.lock() {
            // Best-effort: an audit write failure should not crash a tool call.
            let _ = log.record(entry);
        }
    }
}

/// Builder for [`ToolContext`] with sensible, fully-functional defaults.
///
/// Defaults are deliberately *permissive and in-memory friendly* so a context
/// can be constructed for a workspace with one call. Override any field with the
/// setters before calling [`build`](ToolContextBuilder::build).
pub struct ToolContextBuilder {
    workspace_root: std::path::PathBuf,
    policy: PermissionPolicy,
    command_policy: CommandPolicy,
    budget: ResourceBudget,
    cancel: CancellationToken,
    audit_path: Option<std::path::PathBuf>,
    memory_path: Option<std::path::PathBuf>,
    checkpoint_dir: Option<std::path::PathBuf>,
    fetcher: Option<Arc<dyn Fetcher>>,
    mcp: Option<Arc<dyn McpClient>>,
    approver: Option<Arc<dyn Approver>>,
    hooks: Option<Arc<HookRegistry>>,
    session: SessionId,
}

impl std::fmt::Debug for ToolContextBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolContextBuilder")
            .field("workspace_root", &self.workspace_root)
            .field("session", &self.session)
            .finish_non_exhaustive()
    }
}

impl ToolContextBuilder {
    /// Create a builder for a workspace at `workspace_root`.
    pub fn new(workspace_root: impl AsRef<std::path::Path>) -> Self {
        ToolContextBuilder {
            workspace_root: workspace_root.as_ref().to_path_buf(),
            policy: PermissionPolicy::permissive(),
            command_policy: CommandPolicy::default(),
            budget: ResourceBudget::default(),
            cancel: CancellationToken::new(),
            audit_path: None,
            memory_path: None,
            checkpoint_dir: None,
            fetcher: None,
            mcp: None,
            approver: None,
            hooks: None,
            session: SessionId::new(),
        }
    }

    /// Set the capability permission policy.
    pub fn policy(mut self, policy: PermissionPolicy) -> Self {
        self.policy = policy;
        self
    }

    /// Set the shell-command safety policy.
    pub fn command_policy(mut self, policy: CommandPolicy) -> Self {
        self.command_policy = policy;
        self
    }

    /// Set the per-call resource budget.
    pub fn budget(mut self, budget: ResourceBudget) -> Self {
        self.budget = budget;
        self
    }

    /// Set the cancellation token.
    pub fn cancel(mut self, cancel: CancellationToken) -> Self {
        self.cancel = cancel;
        self
    }

    /// Set the audit log file path (defaults to `<workspace>/.na/audit.jsonl`).
    pub fn audit_path(mut self, path: impl AsRef<std::path::Path>) -> Self {
        self.audit_path = Some(path.as_ref().to_path_buf());
        self
    }

    /// Set the memory store file path (defaults to `<workspace>/.na/memory.jsonl`).
    pub fn memory_path(mut self, path: impl AsRef<std::path::Path>) -> Self {
        self.memory_path = Some(path.as_ref().to_path_buf());
        self
    }

    /// Set the checkpoint store directory (defaults to `<workspace>/.na/checkpoints`).
    pub fn checkpoint_dir(mut self, path: impl AsRef<std::path::Path>) -> Self {
        self.checkpoint_dir = Some(path.as_ref().to_path_buf());
        self
    }

    /// Provide a custom [`Fetcher`] (defaults to [`MockFetcher`]).
    pub fn fetcher(mut self, fetcher: Arc<dyn Fetcher>) -> Self {
        self.fetcher = Some(fetcher);
        self
    }

    /// Provide a custom [`McpClient`] (defaults to [`MockMcpClient`]).
    pub fn mcp(mut self, mcp: Arc<dyn McpClient>) -> Self {
        self.mcp = Some(mcp);
        self
    }

    /// Provide a custom [`Approver`] (defaults to [`AllowAllApprover`]).
    pub fn approver(mut self, approver: Arc<dyn Approver>) -> Self {
        self.approver = Some(approver);
        self
    }

    /// Provide the tool-lifecycle [`HookRegistry`] (defaults to an empty one).
    pub fn hooks(mut self, hooks: Arc<HookRegistry>) -> Self {
        self.hooks = Some(hooks);
        self
    }

    /// Set the session id (defaults to a fresh one).
    pub fn session(mut self, session: SessionId) -> Self {
        self.session = session;
        self
    }

    /// Materialize the context: canonicalize the jail, open the stores, and wire
    /// in defaults for any unset component.
    pub fn build(self) -> Result<ToolContext> {
        let jail = PathJail::new(&self.workspace_root)?;

        // All internal stores live under a hidden ".na" dir in the workspace so
        // they survive restarts without polluting the user's tree.
        let na_dir = jail.root().join(".na");
        std::fs::create_dir_all(&na_dir)
            .map_err(|e| CoreError::from(e).with_context("creating .na state dir"))?;

        let audit_path = self
            .audit_path
            .unwrap_or_else(|| na_dir.join("audit.jsonl"));
        let memory_path = self
            .memory_path
            .unwrap_or_else(|| na_dir.join("memory.jsonl"));
        // Default the checkpoint store to the whole ".na" dir so the snapshot
        // walker (which skips its own store dir) excludes ALL internal state —
        // audit log, memory store, and checkpoint blobs alike. This keeps a
        // workspace restore scoped to the user's manuscript: rolling back the
        // prose never wipes long-term memory or the audit trail.
        let checkpoint_dir = self.checkpoint_dir.unwrap_or_else(|| na_dir.clone());

        let audit = AuditLog::open(&audit_path)?;
        let memory = MemoryStore::open(&memory_path)?;
        let checkpoints = CheckpointStore::open(jail.root(), &checkpoint_dir)?;

        Ok(ToolContext {
            jail,
            policy: self.policy,
            command_policy: self.command_policy,
            budget: self.budget,
            cancel: self.cancel,
            audit: Arc::new(std::sync::Mutex::new(audit)),
            memory: Arc::new(std::sync::Mutex::new(memory)),
            checkpoints: Arc::new(std::sync::Mutex::new(checkpoints)),
            fetcher: self.fetcher.unwrap_or_else(|| Arc::new(MockFetcher::new())),
            mcp: self.mcp.unwrap_or_else(|| Arc::new(MockMcpClient::new())),
            approver: self.approver.unwrap_or_else(|| Arc::new(AllowAllApprover)),
            hooks: self.hooks.unwrap_or_else(|| Arc::new(HookRegistry::new())),
            session: self.session,
        })
    }
}

/// A tool: a named, schema'd, capability-gated unit of work the agent can call.
///
/// `execute` returns a [`BoxFuture`] (not `async fn`) to keep the trait
/// object-safe; implementors write `Box::pin(async move { ... })`.
pub trait Tool: Send + Sync {
    /// The tool's static specification.
    fn spec(&self) -> ToolSpec;

    /// Run the tool with validated `args` against the shared `ctx`.
    fn execute<'a>(&'a self, args: Json, ctx: &'a ToolContext)
        -> BoxFuture<'a, Result<ToolResult>>;
}

/// An outbound content fetcher (HTTP, etc.). Object-safe; the default
/// implementation is [`MockFetcher`] so the crate never performs real network IO
/// in tests.
pub trait Fetcher: Send + Sync {
    /// Fetch the body at `url` as text.
    fn fetch<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<String>>;
}

/// A [`Fetcher`] returning canned responses keyed by exact URL.
#[derive(Debug, Default)]
pub struct MockFetcher {
    responses: std::collections::HashMap<String, String>,
}

impl MockFetcher {
    /// Create an empty mock (every URL is "not found").
    pub fn new() -> Self {
        MockFetcher {
            responses: std::collections::HashMap::new(),
        }
    }

    /// Register a canned `body` for an exact `url` (builder style).
    pub fn with(mut self, url: impl Into<String>, body: impl Into<String>) -> Self {
        self.responses.insert(url.into(), body.into());
        self
    }

    /// Insert a canned response in place.
    pub fn insert(&mut self, url: impl Into<String>, body: impl Into<String>) {
        self.responses.insert(url.into(), body.into());
    }
}

impl Fetcher for MockFetcher {
    fn fetch<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<String>> {
        let result = self
            .responses
            .get(url)
            .cloned()
            .ok_or_else(|| CoreError::not_found(format!("no mock response for {url}")));
        Box::pin(async move { result })
    }
}

/// A Model-Context-Protocol client exposing remote tools. Object-safe; the
/// default implementation is [`MockMcpClient`].
pub trait McpClient: Send + Sync {
    /// List the tools the remote server exposes.
    fn list_tools<'a>(&'a self) -> BoxFuture<'a, Result<Vec<ToolSpec>>>;

    /// Invoke a remote tool by `name` with JSON `args`, returning its raw result.
    fn call_tool<'a>(&'a self, name: &'a str, args: Json) -> BoxFuture<'a, Result<Json>>;
}

/// A [`McpClient`] with a configurable set of fake remote tools.
#[derive(Debug, Default)]
pub struct MockMcpClient {
    tools: Vec<ToolSpec>,
    responses: std::collections::HashMap<String, Json>,
}

impl MockMcpClient {
    /// Create an empty mock with no remote tools.
    pub fn new() -> Self {
        MockMcpClient {
            tools: Vec::new(),
            responses: std::collections::HashMap::new(),
        }
    }

    /// Register a remote tool spec and the JSON it returns when called.
    pub fn with_tool(mut self, spec: ToolSpec, response: Json) -> Self {
        self.responses.insert(spec.name.clone(), response);
        self.tools.push(spec);
        self
    }
}

impl McpClient for MockMcpClient {
    fn list_tools<'a>(&'a self) -> BoxFuture<'a, Result<Vec<ToolSpec>>> {
        let tools = self.tools.clone();
        Box::pin(async move { Ok(tools) })
    }

    fn call_tool<'a>(&'a self, name: &'a str, _args: Json) -> BoxFuture<'a, Result<Json>> {
        let resp = self
            .responses
            .get(name)
            .cloned()
            .ok_or_else(|| CoreError::not_found(format!("no mock MCP tool {name}")));
        Box::pin(async move { resp })
    }
}

/// Stamp `result` with the elapsed time since `started_ms`.
pub(crate) fn stamp_duration(mut result: ToolResult, started_ms: u64) -> ToolResult {
    result.metadata.duration_ms = now_millis().saturating_sub(started_ms);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use na_common::json;

    fn temp_root(tag: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("na_tools_ctx_{}_{}", tag, na_common::next_id("t")));
        p
    }

    #[test]
    fn tool_result_success_and_error_shapes() {
        let ok = ToolResult::success("hello", json!({"n": 1}));
        assert!(ok.ok);
        assert_eq!(ok.content, "hello");
        assert_eq!(ok.metadata.bytes, 5);

        let err = CoreError::not_found("gone");
        let res = ToolResult::from_error(&err);
        assert!(!res.ok);
        assert!(res.content.starts_with("[error:not_found]"));
        assert_eq!(res.data["code"], "not_found");
    }

    #[test]
    fn spec_round_trips_json() {
        let spec = ToolSpec::new(
            "read_file",
            "Read a file",
            json!({"type": "object"}),
            vec![Capability::ReadFile],
            false,
        );
        let s = serde_json::to_string(&spec).unwrap();
        let back: ToolSpec = serde_json::from_str(&s).unwrap();
        assert_eq!(spec, back);
    }

    #[test]
    fn spec_old_json_defaults_concurrency() {
        let raw = r#"{
            "name": "legacy",
            "description": "old spec",
            "input_schema": { "type": "object" },
            "capabilities": [],
            "mutating": true
        }"#;
        let spec: ToolSpec = serde_json::from_str(raw).unwrap();
        // Old persisted specs had only `mutating`; missing concurrency is a safe
        // serialized default and does not affect live tool registrations.
        assert_eq!(spec.concurrency, ToolConcurrency::ReadOnly);
        assert!(spec.mutating);
    }

    #[test]
    fn spec_marks_subagent_concurrency() {
        let spec = ToolSpec::new("spawn_subagent", "spawn", json!({}), vec![], true)
            .with_subagent_concurrency();
        assert!(spec.mutating);
        assert_eq!(spec.concurrency, ToolConcurrency::Subagent);
    }

    #[test]
    fn builder_defaults_build_ok() {
        let ctx = ToolContextBuilder::new(temp_root("build")).build().unwrap();
        // permissive default policy allows everything
        assert!(ctx.require(Capability::WriteFile, "anything").is_ok());
        assert!(ctx.session.as_str().starts_with("sess_"));
    }

    #[test]
    fn require_allow_deny_ask() {
        let root = temp_root("req");
        // Deny writes, ask for reads, allow listing.
        let policy = PermissionPolicy::restrictive()
            .allow(Capability::ListDir, "**")
            .ask(Capability::ReadFile, "**");
        let ctx = ToolContextBuilder::new(&root)
            .policy(policy)
            .approver(Arc::new(DenyAllApprover))
            .build()
            .unwrap();

        assert!(ctx.require(Capability::ListDir, "x").is_ok());
        // Ask + DenyAllApprover -> denied.
        let e = ctx.require(Capability::ReadFile, "x").unwrap_err();
        assert!(e.is(na_common::ErrorKind::PermissionDenied));
        // No rule, restrictive default -> denied.
        assert!(ctx.require(Capability::WriteFile, "x").is_err());
    }

    #[test]
    fn ask_with_allow_all_approver_passes() {
        let root = temp_root("ask");
        let policy = PermissionPolicy::ask_by_default();
        let ctx = ToolContextBuilder::new(&root)
            .policy(policy)
            .approver(Arc::new(AllowAllApprover))
            .build()
            .unwrap();
        assert!(ctx.require(Capability::WriteFile, "x").is_ok());
    }

    #[tokio::test]
    async fn mock_fetcher_returns_canned() {
        let f = MockFetcher::new().with("http://x/", "<p>hi</p>");
        assert_eq!(f.fetch("http://x/").await.unwrap(), "<p>hi</p>");
        assert!(f.fetch("http://missing/").await.is_err());
    }

    #[tokio::test]
    async fn mock_mcp_lists_and_calls() {
        let spec = ToolSpec::new(
            "remote_echo",
            "echo",
            json!({"type":"object"}),
            vec![],
            false,
        );
        let mcp = MockMcpClient::new().with_tool(spec.clone(), json!({"echoed": true}));
        let tools = mcp.list_tools().await.unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "remote_echo");
        let r = mcp.call_tool("remote_echo", json!({})).await.unwrap();
        assert_eq!(r["echoed"], true);
        assert!(mcp.call_tool("nope", json!({})).await.is_err());
    }

    #[test]
    fn context_is_clone_and_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<ToolContext>();
        let ctx = ToolContextBuilder::new(temp_root("clone")).build().unwrap();
        let _c2 = ctx.clone();
    }
}
