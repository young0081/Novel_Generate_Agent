//! The tool registry and the complete tool-call lifecycle.
//!
//! [`ToolRegistry`] maps tool names to `Arc<dyn Tool>`. Its
//! [`invoke`](ToolRegistry::invoke) method is the single entry point through
//! which the runtime runs a tool, and it enforces the full lifecycle in order:
//!
//! 1. **Lookup** — unknown tool ⇒ `NotFound` error result.
//! 2. **Validate** — `args` are checked against the tool's `input_schema`.
//! 3. **Authorize** — every declared [`Capability`] is run through
//!    [`ToolContext::require`].
//! 4. **Cancellation** — bail out early if the token is already cancelled.
//! 5. **Pre-tool hooks** — [`ctx.hooks`](ToolContext::hooks) may
//!    [`Block`](crate::HookDecision::Block) the call (a `security_blocked` result
//!    with the tool *not* run) or [`Replace`](crate::HookDecision::Replace) it
//!    with a canned result.
//! 6. **Execute under deadline** — the tool future races a
//!    [`tokio::time::timeout`] (the budget's wall clock) *and* the cancellation
//!    token, so a hung or cancelled tool cannot run forever.
//! 7. **Normalize** — any `Err(CoreError)` becomes an error-first
//!    [`ToolResult`]; a panic-free path guarantees we always return a result.
//! 8. **Post-tool hooks** — the result is folded through every hook's
//!    `post_tool` for redaction/annotation.
//! 9. **Audit** — every call writes a `tool_call` [`AuditEntry`].
//!
//! [`invoke`](ToolRegistry::invoke) never panics and never returns `Err`; it
//! always yields a [`ToolResult`] (with `ok = false` on failure).

use std::collections::BTreeMap;
use std::sync::Arc;

use na_common::time::now_millis;
use na_common::{CoreError, Json, Result};
use na_memory::AuditEntry;

use crate::hooks::HookDecision;
use crate::tool::{stamp_duration, Tool, ToolContext, ToolResult, ToolSpec};
use crate::validate::validate;

/// A name → tool map with a guarded invocation lifecycle.
#[derive(Default, Clone)]
pub struct ToolRegistry {
    tools: BTreeMap<String, Arc<dyn Tool>>,
}

impl std::fmt::Debug for ToolRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolRegistry")
            .field("tools", &self.tools.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl ToolRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        ToolRegistry {
            tools: BTreeMap::new(),
        }
    }

    /// Register a tool (keyed by its spec name). Returns an error if a tool with
    /// the same name is already registered.
    pub fn register(&mut self, tool: Arc<dyn Tool>) -> Result<()> {
        let name = tool.spec().name;
        if self.tools.contains_key(&name) {
            return Err(CoreError::conflict(format!(
                "tool {name:?} is already registered"
            )));
        }
        self.tools.insert(name, tool);
        Ok(())
    }

    /// Register a tool, replacing any existing tool with the same name.
    pub fn register_or_replace(&mut self, tool: Arc<dyn Tool>) {
        let name = tool.spec().name;
        self.tools.insert(name, tool);
    }

    /// Look up a tool by name.
    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).cloned()
    }

    /// Whether a tool with this name is registered.
    pub fn contains(&self, name: &str) -> bool {
        self.tools.contains_key(name)
    }

    /// Number of registered tools.
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    /// Whether the registry has no tools.
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    /// The specs of every registered tool, sorted by name.
    pub fn list_specs(&self) -> Vec<ToolSpec> {
        self.tools.values().map(|t| t.spec()).collect()
    }

    /// The names of every registered tool, sorted.
    pub fn names(&self) -> Vec<String> {
        self.tools.keys().cloned().collect()
    }

    /// The complete, guarded tool-call lifecycle. Always returns a
    /// [`ToolResult`] (never `Err`, never panics).
    pub async fn invoke(&self, name: &str, args: Json, ctx: &ToolContext) -> ToolResult {
        let started = now_millis();

        // i. lookup.
        let Some(tool) = self.get(name) else {
            let err = CoreError::not_found(format!("unknown tool {name:?}"));
            self.audit(ctx, name, &Err::<(), _>(err.clone()), None);
            return stamp_duration(ToolResult::from_error(&err), started);
        };
        let spec = tool.spec();

        // ii. validate args against the schema.
        if let Err(e) = validate(&spec.input_schema, &args) {
            self.audit(ctx, name, &Err::<(), _>(e.clone()), None);
            return stamp_duration(ToolResult::from_error(&e), started);
        }

        // iii. authorize every declared capability.
        let resource = resource_hint(&args, name);
        for cap in &spec.capabilities {
            if let Err(e) = ctx.require(*cap, &resource) {
                self.audit(ctx, name, &Err::<(), _>(e.clone()), Some("deny"));
                return stamp_duration(ToolResult::from_error(&e), started);
            }
        }

        // iv. honor an already-cancelled token.
        if let Err(e) = ctx.cancel.check() {
            self.audit(ctx, name, &Err::<(), _>(e.clone()), None);
            return stamp_duration(ToolResult::from_error(&e), started);
        }

        // v. pre-tool hooks: a hook may block the call (security) or replace its
        // result outright, in which case the tool itself never runs.
        match ctx.hooks.run_pre(name, &args, ctx) {
            HookDecision::Proceed => {}
            HookDecision::Block(reason) => {
                let e = CoreError::security(format!("tool {name:?} blocked by hook: {reason}"));
                self.audit(ctx, name, &Err::<(), _>(e.clone()), Some("hook_block"));
                let blocked = ToolResult::from_error(&e);
                // Still let post hooks observe the (blocked) outcome.
                let blocked = ctx.hooks.run_post(name, &args, blocked, ctx);
                return stamp_duration(blocked, started);
            }
            HookDecision::Replace(result) => {
                self.audit(ctx, name, &Ok::<_, CoreError>(()), Some("hook_replace"));
                let replaced = ctx.hooks.run_post(name, &args, result, ctx);
                return stamp_duration(replaced, started);
            }
        }

        // vi. execute under a wall-clock deadline, racing cancellation.
        let result = run_guarded(tool.as_ref(), args.clone(), ctx).await;

        // vii. normalize + viii. post-tool hooks + ix. audit.
        match result {
            Ok(r) => {
                let r = ctx.hooks.run_post(name, &args, r, ctx);
                self.audit(ctx, name, &Ok::<_, CoreError>(()), Some("allow"));
                stamp_duration(r, started)
            }
            Err(e) => {
                let err_result = ctx
                    .hooks
                    .run_post(name, &args, ToolResult::from_error(&e), ctx);
                self.audit(ctx, name, &Err::<(), _>(e.clone()), Some("allow"));
                stamp_duration(err_result, started)
            }
        }
    }

    /// Write a `tool_call` audit entry summarizing the outcome.
    fn audit<T>(
        &self,
        ctx: &ToolContext,
        name: &str,
        outcome: &std::result::Result<T, CoreError>,
        decision: Option<&str>,
    ) {
        let mut entry = AuditEntry::new("tool_call")
            .tool(name)
            .session(ctx.session.as_str());
        if let Some(d) = decision {
            entry = entry.decision(d);
        }
        match outcome {
            Ok(_) => entry = entry.ok(true),
            Err(e) => entry = entry.from_error(e),
        }
        ctx.audit_record(entry);
    }
}

/// Run the tool future under the budget's wall-clock deadline, also aborting if
/// the cancellation token fires. Returns the tool's `Result`, mapping a timeout
/// to a `Timeout` error and a cancel to a `Cancelled` error.
async fn run_guarded(tool: &dyn Tool, args: Json, ctx: &ToolContext) -> Result<ToolResult> {
    let deadline = ctx.budget.wall_duration();
    let fut = tool.execute(args, ctx);
    let cancel = ctx.cancel.clone();

    tokio::select! {
        biased;
        // Cancellation wins if it fires first.
        _ = cancel.cancelled() => {
            Err(CoreError::cancelled("tool execution cancelled"))
        }
        // Otherwise run under the timeout.
        res = tokio::time::timeout(deadline, fut) => {
            match res {
                Ok(inner) => inner,
                Err(_elapsed) => Err(CoreError::timeout(format!(
                    "tool exceeded wall-clock budget of {} ms",
                    ctx.budget.max_wall_ms
                ))),
            }
        }
    }
}

/// Best-effort resource string for capability checks, derived from common arg
/// shapes (a `path`/`url`/`query` field) falling back to the tool name.
fn resource_hint(args: &Json, name: &str) -> String {
    for key in ["path", "url", "file", "query", "name", "command"] {
        if let Some(v) = args.get(key).and_then(Json::as_str) {
            return v.to_string();
        }
    }
    name.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool::{BoxFuture, ToolContextBuilder};
    use na_common::json;
    use na_sandbox::{Capability, PermissionPolicy};
    use std::sync::Arc;

    fn temp_root(tag: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("na_tools_reg_{}_{}", tag, na_common::next_id("t")));
        p
    }

    /// A trivial tool that echoes its `message` argument.
    struct EchoTool;
    impl Tool for EchoTool {
        fn spec(&self) -> ToolSpec {
            ToolSpec::new(
                "echo",
                "Echo a message",
                json!({
                    "type": "object",
                    "required": ["message"],
                    "properties": { "message": { "type": "string" } }
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
                let msg = args["message"].as_str().unwrap_or("").to_string();
                Ok(ToolResult::success(msg.clone(), json!({ "echoed": msg })))
            })
        }
    }

    /// A tool that requires WriteFile and sleeps forever (to test cancel/timeout).
    struct SleepTool;
    impl Tool for SleepTool {
        fn spec(&self) -> ToolSpec {
            ToolSpec::new(
                "sleep",
                "Sleep forever",
                json!({ "type": "object" }),
                vec![Capability::WriteFile],
                true,
            )
        }
        fn execute<'a>(
            &'a self,
            _args: Json,
            _ctx: &'a ToolContext,
        ) -> BoxFuture<'a, Result<ToolResult>> {
            Box::pin(async move {
                tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
                Ok(ToolResult::success("done", json!({})))
            })
        }
    }

    fn registry() -> ToolRegistry {
        let mut r = ToolRegistry::new();
        r.register(Arc::new(EchoTool)).unwrap();
        r.register(Arc::new(SleepTool)).unwrap();
        r
    }

    #[tokio::test]
    async fn invoke_happy_path() {
        let ctx = ToolContextBuilder::new(temp_root("happy")).build().unwrap();
        let r = registry();
        let res = r.invoke("echo", json!({ "message": "hi" }), &ctx).await;
        assert!(res.ok);
        assert_eq!(res.content, "hi");
        assert_eq!(res.data["echoed"], "hi");

        // Audited as a successful tool_call.
        let log = ctx.audit.lock().unwrap();
        let entries = log
            .query(
                na_memory::AuditFilter::new()
                    .event("tool_call")
                    .tool("echo"),
            )
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].ok);
    }

    #[tokio::test]
    async fn unknown_tool_is_not_found_result() {
        let ctx = ToolContextBuilder::new(temp_root("unknown"))
            .build()
            .unwrap();
        let r = registry();
        let res = r.invoke("does_not_exist", json!({}), &ctx).await;
        assert!(!res.ok);
        assert!(res.content.contains("[error:not_found]"));
        assert_eq!(res.data["code"], "not_found");
    }

    #[tokio::test]
    async fn bad_args_yield_invalid_input_and_audited() {
        let ctx = ToolContextBuilder::new(temp_root("badargs"))
            .build()
            .unwrap();
        let r = registry();
        // Missing required "message".
        let res = r.invoke("echo", json!({}), &ctx).await;
        assert!(!res.ok);
        assert_eq!(res.data["code"], "invalid_input");

        let log = ctx.audit.lock().unwrap();
        let entries = log
            .query(na_memory::AuditFilter::new().tool("echo").ok(false))
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].error_code.as_deref(), Some("invalid_input"));
    }

    #[tokio::test]
    async fn denied_capability_yields_permission_denied() {
        // Restrictive policy denies WriteFile (no allow rule).
        let ctx = ToolContextBuilder::new(temp_root("denied"))
            .policy(PermissionPolicy::restrictive())
            .build()
            .unwrap();
        let r = registry();
        let res = r.invoke("sleep", json!({}), &ctx).await;
        assert!(!res.ok);
        assert_eq!(res.data["code"], "permission_denied");

        let log = ctx.audit.lock().unwrap();
        let denied = log
            .query(na_memory::AuditFilter::new().tool("sleep").ok(false))
            .unwrap();
        assert_eq!(denied.len(), 1);
        assert_eq!(denied[0].decision.as_deref(), Some("deny"));
    }

    #[tokio::test]
    async fn cancelled_token_aborts() {
        let cancel = na_common::CancellationToken::new();
        cancel.cancel(); // already cancelled
        let ctx = ToolContextBuilder::new(temp_root("cancel"))
            .cancel(cancel)
            .build()
            .unwrap();
        let r = registry();
        let res = r.invoke("sleep", json!({}), &ctx).await;
        assert!(!res.ok);
        assert_eq!(res.data["code"], "cancelled");
    }

    #[tokio::test]
    async fn timeout_aborts_hung_tool() {
        let budget = na_sandbox::ResourceBudget::new(64 * 1024, 50 /*ms*/, 50);
        let ctx = ToolContextBuilder::new(temp_root("timeout"))
            .budget(budget)
            .build()
            .unwrap();
        let r = registry();
        let res = r.invoke("sleep", json!({}), &ctx).await;
        assert!(!res.ok);
        assert_eq!(res.data["code"], "timeout");
    }

    #[tokio::test]
    async fn cancel_midflight_aborts() {
        let cancel = na_common::CancellationToken::new();
        let ctx = ToolContextBuilder::new(temp_root("midflight"))
            .cancel(cancel.clone())
            .build()
            .unwrap();
        let r = registry();
        let handle = {
            let r = r.clone();
            let ctx = ctx.clone();
            tokio::spawn(async move { r.invoke("sleep", json!({}), &ctx).await })
        };
        tokio::task::yield_now().await;
        cancel.cancel();
        let res = tokio::time::timeout(std::time::Duration::from_secs(5), handle)
            .await
            .expect("invoke should return quickly after cancel")
            .unwrap();
        assert!(!res.ok);
        assert_eq!(res.data["code"], "cancelled");
    }

    #[test]
    fn duplicate_registration_conflicts() {
        let mut r = ToolRegistry::new();
        r.register(Arc::new(EchoTool)).unwrap();
        let err = r.register(Arc::new(EchoTool)).unwrap_err();
        assert!(err.is(na_common::ErrorKind::Conflict));
    }

    #[test]
    fn list_specs_and_names() {
        let r = registry();
        let names = r.names();
        assert_eq!(names, vec!["echo".to_string(), "sleep".to_string()]);
        assert_eq!(r.list_specs().len(), 2);
    }

    // ---- hook integration ----

    use crate::hooks::{DenyToolHook, HookRegistry, ToolHook};
    use crate::tool::ToolResult;

    fn ctx_with_hooks(tag: &str, hooks: HookRegistry) -> ToolContext {
        ToolContextBuilder::new(temp_root(tag))
            .hooks(Arc::new(hooks))
            .build()
            .unwrap()
    }

    #[tokio::test]
    async fn deny_hook_blocks_tool_without_executing() {
        // A "trap" tool that would panic if it ever ran.
        struct TrapTool;
        impl Tool for TrapTool {
            fn spec(&self) -> ToolSpec {
                ToolSpec::new(
                    "trap",
                    "must not run",
                    json!({ "type": "object" }),
                    vec![],
                    false,
                )
            }
            fn execute<'a>(
                &'a self,
                _args: Json,
                _ctx: &'a ToolContext,
            ) -> BoxFuture<'a, Result<ToolResult>> {
                Box::pin(async move { panic!("trap tool must never execute when blocked") })
            }
        }
        let mut r = ToolRegistry::new();
        r.register(Arc::new(TrapTool)).unwrap();

        let mut hooks = HookRegistry::new();
        hooks.register(Arc::new(DenyToolHook::new(["trap"])));
        let ctx = ctx_with_hooks("denyhook", hooks);

        let res = r.invoke("trap", json!({}), &ctx).await;
        assert!(!res.ok);
        assert_eq!(res.data["code"], "security_blocked");
        assert!(res.content.contains("[error:security_blocked]"));

        // Audited with the hook_block decision.
        let log = ctx.audit.lock().unwrap();
        let entries = log
            .query(na_memory::AuditFilter::new().tool("trap").ok(false))
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].decision.as_deref(), Some("hook_block"));
    }

    /// A pre hook that replaces the call with a canned success.
    struct ReplaceHook;
    impl ToolHook for ReplaceHook {
        fn name(&self) -> &str {
            "replace"
        }
        fn pre_tool(
            &self,
            _tool: &str,
            _args: &Json,
            _ctx: &ToolContext,
        ) -> crate::hooks::HookDecision {
            crate::hooks::HookDecision::Replace(ToolResult::success(
                "replaced!",
                json!({ "replaced": true }),
            ))
        }
    }

    #[tokio::test]
    async fn replace_hook_returns_canned_result() {
        let r = registry();
        let mut hooks = HookRegistry::new();
        hooks.register(Arc::new(ReplaceHook));
        let ctx = ctx_with_hooks("replacehook", hooks);

        let res = r
            .invoke("echo", json!({ "message": "ignored" }), &ctx)
            .await;
        assert!(res.ok);
        assert_eq!(res.content, "replaced!");
        assert_eq!(res.data["replaced"], true);

        let log = ctx.audit.lock().unwrap();
        let entries = log
            .query(na_memory::AuditFilter::new().tool("echo"))
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].decision.as_deref(), Some("hook_replace"));
    }

    /// A post hook that uppercases the content.
    struct UpperHook;
    impl ToolHook for UpperHook {
        fn name(&self) -> &str {
            "upper"
        }
        fn post_tool(
            &self,
            _tool: &str,
            _args: &Json,
            mut result: ToolResult,
            _ctx: &ToolContext,
        ) -> ToolResult {
            result.content = result.content.to_uppercase();
            result
        }
    }

    #[tokio::test]
    async fn post_hook_transforms_content() {
        let r = registry();
        let mut hooks = HookRegistry::new();
        hooks.register(Arc::new(UpperHook));
        let ctx = ctx_with_hooks("posthook", hooks);

        let res = r.invoke("echo", json!({ "message": "hi" }), &ctx).await;
        assert!(res.ok);
        assert_eq!(res.content, "HI");
    }

    #[tokio::test]
    async fn no_hooks_means_unchanged_behavior() {
        // Default context has an empty hook registry; behavior matches the plain
        // happy path.
        let ctx = ToolContextBuilder::new(temp_root("nohooks"))
            .build()
            .unwrap();
        let r = registry();
        let res = r.invoke("echo", json!({ "message": "plain" }), &ctx).await;
        assert!(res.ok);
        assert_eq!(res.content, "plain");
    }
}
