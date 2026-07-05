//! The [`Engine`] — a thin, GUI-facing facade over the whole core.
//!
//! It owns a [`ToolRegistry`] pre-populated with the built-in (and optionally the
//! runtime) tools, and a shared [`ToolContext`] (workspace jail, permission
//! policy, memory / checkpoint / audit stores, lifecycle hooks). The desktop
//! (Electron) and mobile (Flutter) shells talk to one of these — either in-process
//! via these methods, or over the JSON-RPC protocol in [`crate::rpc`].
//!
//! Two constructors:
//! * [`Engine::new`] — the base engine: every built-in tool, empty hooks.
//! * [`Engine::full`] — additionally wires lifecycle hooks, a skill registry, and
//!   the runtime tools (`skill_list`, `skill_load`, `spawn_subagent`) backed by a
//!   model provider, plus loop-observability hooks.
//!
//! Either way, [`run_goal_*`](Engine::run_goal_with) injects the project profile
//! (`writer.md` / `outline.md`) as system steering at the start of every run.

use std::path::Path;
use std::sync::Arc;

use na_common::{json, Json, Result};
use na_runtime::{
    register_runtime_tools, CompletionResponse, GoalLoop, LoopHookRegistry, LoopOutcome,
    MockProvider, ModelProvider, ProjectProfile, Protocol, Session, SkillRegistry,
};
use na_tools::{
    builtin_registry, HookRegistry, ToolContext, ToolContextBuilder, ToolRegistry, ToolResult,
    ToolSpec,
};

/// The core engine: registry + shared tool context + optional loop hooks.
pub struct Engine {
    /// Every registered tool, keyed by name.
    pub registry: ToolRegistry,
    /// The shared execution context (jail, policy, stores, hooks, cancellation).
    pub ctx: ToolContext,
    /// Observability hooks fired around each agent-loop step (default empty).
    pub loop_hooks: Arc<LoopHookRegistry>,
}

impl Engine {
    /// Build the base engine rooted at `workspace_root`, opening the on-disk
    /// stores under `<workspace_root>/.na/`. Registers the built-in tools and uses
    /// empty hooks.
    pub fn new(workspace_root: impl AsRef<Path>) -> Result<Self> {
        Ok(Engine {
            registry: builtin_registry(),
            ctx: ToolContextBuilder::new(workspace_root).build()?,
            loop_hooks: Arc::new(LoopHookRegistry::new()),
        })
    }

    /// Build a fully-wired engine: built-in tools **plus** the runtime tools
    /// (`skill_list`, `skill_load`, `spawn_subagent`), lifecycle `hooks` on the
    /// tool context, a `skills` registry, a `subagent_provider` that powers
    /// delegated child runs, and `loop_hooks` for step observability.
    pub fn full(
        workspace_root: impl AsRef<Path>,
        subagent_provider: Arc<dyn ModelProvider>,
        skills: Arc<SkillRegistry>,
        hooks: Arc<HookRegistry>,
        loop_hooks: Arc<LoopHookRegistry>,
    ) -> Result<Self> {
        let ctx = ToolContextBuilder::new(workspace_root)
            .hooks(hooks)
            .build()?;
        let mut registry = builtin_registry();
        register_runtime_tools(&mut registry, subagent_provider, skills);
        Ok(Engine {
            registry,
            ctx,
            loop_hooks,
        })
    }

    /// Build an engine from a pre-configured context (e.g. with a custom policy,
    /// approver, fetcher, or MCP client). Registers only the built-in tools.
    pub fn from_context(ctx: ToolContext) -> Self {
        Engine {
            registry: builtin_registry(),
            ctx,
            loop_hooks: Arc::new(LoopHookRegistry::new()),
        }
    }

    /// The JSON specs of every registered tool (for the model and the UI).
    pub fn list_tools(&self) -> Vec<ToolSpec> {
        self.registry.list_specs()
    }

    /// Run one tool through the full guarded lifecycle (validate → authorize →
    /// hooks(pre) → cancel-check → execute-under-deadline → hooks(post) →
    /// process-output → audit). Never panics; failures come back as an error
    /// [`ToolResult`].
    pub async fn invoke_tool(&self, name: &str, args: Json) -> ToolResult {
        self.registry.invoke(name, args, &self.ctx).await
    }

    /// Drive a goal loop with a deterministic, offline, *scripted* model.
    ///
    /// `responses` is a queue of model completions popped one per step — this is
    /// how the demo and tests exercise a complete agent run without a network.
    /// A live provider plugs in via [`run_goal_with`](Self::run_goal_with).
    pub async fn run_goal_scripted(
        &self,
        goal: &str,
        title: &str,
        protocol: Protocol,
        responses: Vec<CompletionResponse>,
    ) -> Result<(LoopOutcome, Session)> {
        let provider = MockProvider::from_responses(responses);
        self.run_goal_with(goal, title, protocol, &provider).await
    }

    /// Drive a goal loop with any [`ModelProvider`] (the live LLM in later phases).
    ///
    /// The project profile (`writer.md` / `outline.md`) is loaded from the
    /// workspace and injected as system steering before the run, and the engine's
    /// loop hooks are attached.
    pub async fn run_goal_with(
        &self,
        goal: &str,
        title: &str,
        protocol: Protocol,
        provider: &dyn ModelProvider,
    ) -> Result<(LoopOutcome, Session)> {
        let mut session = Session::new(title);

        // Inject the author's standing instructions (writer.md / outline.md).
        for msg in ProjectProfile::load(self.ctx.jail.root()).system_messages() {
            session.push(msg);
        }

        let outcome = GoalLoop::with_protocol(protocol)
            .loop_hooks(self.loop_hooks.clone())
            .run(goal, &mut session, provider, &self.registry, &self.ctx)
            .await?;
        Ok((outcome, session))
    }

    /// Signal cancellation to every in-flight tool / loop sharing this context.
    pub fn cancel(&self) {
        self.ctx.cancel.cancel();
    }

    /// The canonical workspace root.
    pub fn workspace_root(&self) -> &Path {
        self.ctx.jail.root()
    }
}

impl std::fmt::Debug for Engine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Engine")
            .field("tools", &self.registry.len())
            .field("workspace_root", &self.workspace_root())
            .field("loop_hooks", &self.loop_hooks.len())
            .finish_non_exhaustive()
    }
}

/// Render a [`LoopOutcome`] as JSON (it is intentionally not `Serialize` itself).
pub fn outcome_to_json(outcome: &LoopOutcome) -> Json {
    json!({
        "stopped_reason": outcome.stopped_reason.as_str(),
        "success": outcome.stopped_reason.is_success(),
        "steps": outcome.steps,
        "final_answer": outcome.final_answer,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use na_runtime::ToolCallRequest;

    fn temp_root(tag: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "na_host_engine_{}_{}",
            tag,
            na_common::next_id("t")
        ));
        p
    }

    #[tokio::test]
    async fn engine_invokes_tools() {
        let engine = Engine::new(temp_root("inv")).unwrap();
        assert_eq!(engine.registry.len(), 23);

        let w = engine
            .invoke_tool(
                "write_file",
                json!({ "path": "book/ch1.md", "content": "第一章\n剑光如雪。" }),
            )
            .await;
        assert!(w.ok, "{}", w.content);

        let r = engine
            .invoke_tool("read_file", json!({ "path": "book/ch1.md" }))
            .await;
        assert!(r.ok);
        assert!(r.content.contains("剑光如雪"));
    }

    #[tokio::test]
    async fn engine_runs_scripted_goal() {
        let engine = Engine::new(temp_root("goal")).unwrap();
        let responses = vec![
            CompletionResponse::tool_call(ToolCallRequest::new(
                "write_file",
                json!({ "path": "ch1.md", "content": "开篇。" }),
            )),
            CompletionResponse::answer("第一章写好了。"),
        ];
        let (outcome, session) = engine
            .run_goal_scripted("写第一章", "测试书", Protocol::NativeToolCall, responses)
            .await
            .unwrap();
        assert!(outcome.stopped_reason.is_success());
        assert!(!session.is_empty());

        let json = outcome_to_json(&outcome);
        assert_eq!(json["success"], true);
        assert_eq!(json["final_answer"], "第一章写好了。");
    }

    #[tokio::test]
    async fn full_engine_registers_runtime_tools() {
        let provider: Arc<dyn ModelProvider> = Arc::new(MockProvider::from_responses(vec![
            CompletionResponse::answer("ok"),
        ]));
        let engine = Engine::full(
            temp_root("full"),
            provider,
            Arc::new(SkillRegistry::new()),
            Arc::new(HookRegistry::new()),
            Arc::new(LoopHookRegistry::new()),
        )
        .unwrap();
        // 23 built-in + 3 runtime tools.
        assert_eq!(engine.registry.len(), 26);
        assert!(engine.registry.contains("spawn_subagent"));
        assert!(engine.registry.contains("skill_list"));
    }

    #[tokio::test]
    async fn writer_md_is_injected_into_runs() {
        let engine = Engine::new(temp_root("profile")).unwrap();
        // Write a style guide into the workspace.
        std::fs::write(engine.workspace_root().join("writer.md"), "只用短句。").unwrap();

        let (_outcome, session) = engine
            .run_goal_scripted(
                "写点东西",
                "书",
                Protocol::NativeToolCall,
                vec![CompletionResponse::answer("好。")],
            )
            .await
            .unwrap();
        // The first message is the injected style guide.
        assert!(session
            .history()
            .iter()
            .any(|m| m.content.contains("作者风格指南") && m.content.contains("只用短句")));
    }
}
