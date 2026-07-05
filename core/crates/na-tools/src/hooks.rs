//! Tool-lifecycle hooks: pluggable interceptors around every tool call.
//!
//! A [`ToolHook`] can observe (and influence) the registry's invocation
//! lifecycle at two points:
//!
//! * **pre-tool** — after the arguments are validated and the capabilities are
//!   authorized, but *before* the tool executes. A hook may let the call
//!   [`Proceed`](HookDecision::Proceed), [`Block`](HookDecision::Block) it with a
//!   reason (turning it into a `security_blocked` error result without running the
//!   tool), or short-circuit it with a [`Replace`](HookDecision::Replace)ment
//!   [`ToolResult`].
//! * **post-tool** — after the tool returns, every hook may transform the
//!   [`ToolResult`] (e.g. redact, annotate, or log it).
//!
//! Hooks are held behind `Arc` in a [`HookRegistry`], so a registry is cheap to
//! clone and safe to share across tasks. The default behaviour of every hook is
//! a no-op, so implementors override only the half they care about.
//!
//! The example hooks [`LoggingHook`] (records each call to the audit log) and
//! [`DenyToolHook`] (blocks a named set of tools) are ready to use.

use std::collections::HashSet;
use std::sync::Arc;

use na_common::Json;
use na_memory::AuditEntry;

use crate::tool::{ToolContext, ToolResult};

/// The verdict a [`ToolHook::pre_tool`] returns for a pending tool call.
#[derive(Debug, Clone, PartialEq)]
pub enum HookDecision {
    /// Allow the call to execute normally.
    Proceed,
    /// Refuse the call. The reason is surfaced as a `security_blocked` error and
    /// the tool is *not* executed.
    Block(String),
    /// Skip execution and return this result directly (still audited).
    Replace(ToolResult),
}

/// An interceptor around the tool-call lifecycle.
///
/// Object-safe and `Send + Sync` so hooks can be stored as `Arc<dyn ToolHook>`
/// and shared across tasks. Both lifecycle methods default to a no-op, so an
/// implementor overrides only the phase it needs.
pub trait ToolHook: Send + Sync {
    /// A short, stable identifier for the hook (used in audit detail/logs).
    fn name(&self) -> &str;

    /// Called before a tool executes. Default: [`HookDecision::Proceed`].
    ///
    /// `args` are the already-validated arguments; `ctx` is the live context.
    fn pre_tool(&self, _tool: &str, _args: &Json, _ctx: &ToolContext) -> HookDecision {
        HookDecision::Proceed
    }

    /// Called after a tool returns. Default: pass the `result` through unchanged.
    fn post_tool(
        &self,
        _tool: &str,
        _args: &Json,
        result: ToolResult,
        _ctx: &ToolContext,
    ) -> ToolResult {
        result
    }
}

/// An ordered set of [`ToolHook`]s run around every tool call.
///
/// `Clone`-friendly: the hooks live behind `Arc`, so cloning the registry just
/// bumps reference counts. Empty by default (a transparent registry).
#[derive(Clone, Default)]
pub struct HookRegistry {
    hooks: Vec<Arc<dyn ToolHook>>,
}

impl std::fmt::Debug for HookRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HookRegistry")
            .field(
                "hooks",
                &self.hooks.iter().map(|h| h.name()).collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl HookRegistry {
    /// An empty hook registry (no interception).
    pub fn new() -> Self {
        HookRegistry { hooks: Vec::new() }
    }

    /// Append a hook. Hooks run in registration order.
    pub fn register(&mut self, hook: Arc<dyn ToolHook>) {
        self.hooks.push(hook);
    }

    /// Number of registered hooks.
    pub fn len(&self) -> usize {
        self.hooks.len()
    }

    /// Whether there are no hooks.
    pub fn is_empty(&self) -> bool {
        self.hooks.is_empty()
    }

    /// The names of the registered hooks, in order.
    pub fn names(&self) -> Vec<&str> {
        self.hooks.iter().map(|h| h.name()).collect()
    }

    /// Run every pre-tool hook in order. The **first** hook that returns
    /// [`Block`](HookDecision::Block) or [`Replace`](HookDecision::Replace) wins
    /// and short-circuits the rest; otherwise the result is
    /// [`Proceed`](HookDecision::Proceed).
    pub fn run_pre(&self, tool: &str, args: &Json, ctx: &ToolContext) -> HookDecision {
        for hook in &self.hooks {
            match hook.pre_tool(tool, args, ctx) {
                HookDecision::Proceed => continue,
                other => return other,
            }
        }
        HookDecision::Proceed
    }

    /// Fold the `result` through every post-tool hook in order, returning the
    /// final transformed result.
    pub fn run_post(
        &self,
        tool: &str,
        args: &Json,
        mut result: ToolResult,
        ctx: &ToolContext,
    ) -> ToolResult {
        for hook in &self.hooks {
            result = hook.post_tool(tool, args, result, ctx);
        }
        result
    }
}

/// A hook that records every tool call to the audit log under a `hook` event.
///
/// It logs once on `pre_tool` (the intent to run) and once on `post_tool` (the
/// outcome, carrying `ok`). It never changes the decision or the result.
#[derive(Debug, Clone)]
pub struct LoggingHook {
    name: String,
}

impl Default for LoggingHook {
    fn default() -> Self {
        LoggingHook {
            name: "logging".to_string(),
        }
    }
}

impl LoggingHook {
    /// A logging hook with the default name `"logging"`.
    pub fn new() -> Self {
        Self::default()
    }

    /// A logging hook with a custom identifying `name`.
    pub fn named(name: impl Into<String>) -> Self {
        LoggingHook { name: name.into() }
    }
}

impl ToolHook for LoggingHook {
    fn name(&self) -> &str {
        &self.name
    }

    fn pre_tool(&self, tool: &str, _args: &Json, ctx: &ToolContext) -> HookDecision {
        ctx.audit_record(
            AuditEntry::new("hook")
                .tool(tool)
                .session(ctx.session.as_str())
                .decision("pre")
                .detail(na_common::json!({ "hook": self.name, "phase": "pre" })),
        );
        HookDecision::Proceed
    }

    fn post_tool(
        &self,
        tool: &str,
        _args: &Json,
        result: ToolResult,
        ctx: &ToolContext,
    ) -> ToolResult {
        ctx.audit_record(
            AuditEntry::new("hook")
                .tool(tool)
                .session(ctx.session.as_str())
                .decision("post")
                .ok(result.ok)
                .detail(na_common::json!({ "hook": self.name, "phase": "post", "ok": result.ok })),
        );
        result
    }
}

/// A hook that blocks a configured set of tool names.
///
/// Any tool whose name is in `names` is refused at `pre_tool` with a
/// `security_blocked` result; everything else proceeds.
#[derive(Debug, Clone, Default)]
pub struct DenyToolHook {
    /// The set of tool names to block.
    pub names: HashSet<String>,
}

impl DenyToolHook {
    /// Block exactly the given tool names.
    pub fn new(names: impl IntoIterator<Item = impl Into<String>>) -> Self {
        DenyToolHook {
            names: names.into_iter().map(Into::into).collect(),
        }
    }

    /// Add another tool name to block (builder style).
    pub fn deny(mut self, name: impl Into<String>) -> Self {
        self.names.insert(name.into());
        self
    }
}

impl ToolHook for DenyToolHook {
    fn name(&self) -> &str {
        "deny_tool"
    }

    fn pre_tool(&self, tool: &str, _args: &Json, _ctx: &ToolContext) -> HookDecision {
        if self.names.contains(tool) {
            HookDecision::Block(format!("tool {tool:?} is blocked by policy"))
        } else {
            HookDecision::Proceed
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool::{ToolContextBuilder, ToolResult};
    use na_common::json;

    fn temp_root(tag: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "na_tools_hooks_{}_{}",
            tag,
            na_common::next_id("t")
        ));
        p
    }

    fn ctx(tag: &str) -> ToolContext {
        ToolContextBuilder::new(temp_root(tag)).build().unwrap()
    }

    #[test]
    fn empty_registry_proceeds() {
        let reg = HookRegistry::new();
        assert!(reg.is_empty());
        let c = ctx("empty");
        assert_eq!(
            reg.run_pre("anything", &json!({}), &c),
            HookDecision::Proceed
        );
        let r = ToolResult::success("x", json!({}));
        let out = reg.run_post("anything", &json!({}), r.clone(), &c);
        assert_eq!(out, r);
    }

    #[test]
    fn deny_hook_blocks_named_tool_only() {
        let mut reg = HookRegistry::new();
        reg.register(Arc::new(DenyToolHook::new(["shell"])));
        let c = ctx("deny");
        match reg.run_pre("shell", &json!({}), &c) {
            HookDecision::Block(reason) => assert!(reason.contains("shell")),
            other => panic!("expected Block, got {other:?}"),
        }
        // A different tool is unaffected.
        assert_eq!(
            reg.run_pre("read_file", &json!({}), &c),
            HookDecision::Proceed
        );
    }

    /// A hook that replaces the call with a canned result.
    struct ReplaceHook;
    impl ToolHook for ReplaceHook {
        fn name(&self) -> &str {
            "replace"
        }
        fn pre_tool(&self, _tool: &str, _args: &Json, _ctx: &ToolContext) -> HookDecision {
            HookDecision::Replace(ToolResult::success("canned", json!({ "canned": true })))
        }
    }

    #[test]
    fn replace_hook_short_circuits() {
        let mut reg = HookRegistry::new();
        reg.register(Arc::new(ReplaceHook));
        let c = ctx("replace");
        match reg.run_pre("whatever", &json!({}), &c) {
            HookDecision::Replace(r) => {
                assert!(r.ok);
                assert_eq!(r.content, "canned");
            }
            other => panic!("expected Replace, got {other:?}"),
        }
    }

    #[test]
    fn first_block_or_replace_wins() {
        // Order: a no-op logging hook, then deny, then a replace. Deny must win
        // because it comes before the replace hook.
        let mut reg = HookRegistry::new();
        reg.register(Arc::new(LoggingHook::new()));
        reg.register(Arc::new(DenyToolHook::new(["x"])));
        reg.register(Arc::new(ReplaceHook));
        let c = ctx("first");
        assert!(matches!(
            reg.run_pre("x", &json!({}), &c),
            HookDecision::Block(_)
        ));
    }

    /// A post hook that appends a marker to the content.
    struct AppendHook(&'static str);
    impl ToolHook for AppendHook {
        fn name(&self) -> &str {
            "append"
        }
        fn post_tool(
            &self,
            _tool: &str,
            _args: &Json,
            mut result: ToolResult,
            _ctx: &ToolContext,
        ) -> ToolResult {
            result.content.push_str(self.0);
            result
        }
    }

    #[test]
    fn post_hooks_fold_in_order() {
        let mut reg = HookRegistry::new();
        reg.register(Arc::new(AppendHook("-a")));
        reg.register(Arc::new(AppendHook("-b")));
        let c = ctx("fold");
        let out = reg.run_post("t", &json!({}), ToolResult::success("base", json!({})), &c);
        assert_eq!(out.content, "base-a-b");
    }

    #[test]
    fn logging_hook_writes_audit_entries() {
        let mut reg = HookRegistry::new();
        reg.register(Arc::new(LoggingHook::new()));
        let c = ctx("logaudit");
        let _ = reg.run_pre("read_file", &json!({}), &c);
        let _ = reg.run_post(
            "read_file",
            &json!({}),
            ToolResult::success("x", json!({})),
            &c,
        );
        let log = c.audit.lock().unwrap();
        let entries = log
            .query(
                na_memory::AuditFilter::new()
                    .event("hook")
                    .tool("read_file"),
            )
            .unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].decision.as_deref(), Some("pre"));
        assert_eq!(entries[1].decision.as_deref(), Some("post"));
    }

    #[test]
    fn registry_is_clone_and_debug() {
        let mut reg = HookRegistry::new();
        reg.register(Arc::new(LoggingHook::named("L")));
        let clone = reg.clone();
        assert_eq!(clone.len(), 1);
        assert_eq!(clone.names(), vec!["L"]);
        let dbg = format!("{reg:?}");
        assert!(dbg.contains("HookRegistry"));
        assert!(dbg.contains('L'));
    }

    #[test]
    fn deny_tool_builder_deny() {
        let hook = DenyToolHook::default().deny("a").deny("b");
        assert!(hook.names.contains("a"));
        assert!(hook.names.contains("b"));
        assert_eq!(hook.name(), "deny_tool");
    }
}
