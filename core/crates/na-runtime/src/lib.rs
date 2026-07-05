//! `na-runtime` — the agent runtime layer of the Novel Generate Team core.
//!
//! This crate is the brain that turns the tool-execution layer ([`na_tools`]),
//! the durable stores ([`na_memory`]), and the safety perimeter ([`na_sandbox`])
//! into a working, *bounded*, goal-directed agent. It is built around one
//! centrepiece — the [`GoalLoop`] — and the supporting machinery it needs:
//!
//! * **Messages & sessions** ([`message`], [`session`]) — the conversation atoms
//!   and a persistable [`Session`] (full serde round-trip to / from disk).
//!
//! * **Context management & compression** ([`context`]) — a [`ContextManager`]
//!   that windows history to a token budget (always keeping system messages) and
//!   [`compress`](ContextManager::compress)es overflow into long-term
//!   [`MemoryStore`](na_memory::MemoryStore), wiring context compression into the
//!   RAG memory. Summaries are produced by the pluggable, offline
//!   [`HeuristicSummarizer`].
//!
//! * **Model provider & orchestration** ([`model`], [`orchestrator`]) — the
//!   object-safe [`ModelProvider`] trait with a scriptable [`MockProvider`], and
//!   an [`Orchestrator`] that assembles [`CompletionRequest`]s from a windowed
//!   context + tool catalog and parses responses into [`AgentAction`]s for both
//!   the native-tool-call and ReAct protocols.
//!
//! * **ReAct protocol** ([`react`]) — a tolerant [`parse_react`] for the
//!   Thought / Action / Action Input / Final Answer format and a matching
//!   [`render_react_system`] preamble.
//!
//! * **Prompt-injection guard** ([`injection`]) — a [`PromptInjectionGuard`] that
//!   detects classic injections and neutralizes untrusted tool output before it
//!   re-enters the model context.
//!
//! * **Scheduling** ([`scheduler`]) — a [`ToolScheduler`] that runs read-only
//!   tools concurrently and serializes mutating ones, honoring cancellation and
//!   per-call timeouts.
//!
//! * **The agent loop** ([`agent_loop`]) — [`GoalLoop`], a conditional,
//!   self-bounding loop that *cannot* spin forever: a [`LoopGuard`] enforces step,
//!   wall-clock, token, repeated-action, and no-progress limits, ending cleanly
//!   with a [`StoppedReason`].
//!
//! ## Object safety
//!
//! Rust's async-fn-in-trait is not `dyn`-compatible, so every trait the runtime
//! needs behind a trait object ([`ModelProvider`], [`Summarizer`](context::Summarizer))
//! returns a manual boxed future ([`BoxFuture`]) and is `Send + Sync`.
//!
//! ```no_run
//! use na_runtime::{GoalLoop, MockProvider, CompletionResponse, Protocol, Session};
//! use na_runtime::message::ToolCallRequest;
//! use na_tools::{builtin_registry, ToolContextBuilder};
//! use na_common::json;
//!
//! # async fn demo() -> na_common::Result<()> {
//! let provider = MockProvider::from_responses(vec![
//!     CompletionResponse::tool_call(ToolCallRequest::new(
//!         "write_file",
//!         json!({ "path": "ch1.md", "content": "第一章" }),
//!     )),
//!     CompletionResponse::answer("第一章已写好。"),
//! ]);
//! let registry = builtin_registry();
//! let ctx = ToolContextBuilder::new("./workspace").build()?;
//! let mut session = Session::new("我的小说");
//!
//! let outcome = GoalLoop::with_protocol(Protocol::NativeToolCall)
//!     .max_steps(8)
//!     .run("写第一章", &mut session, &provider, &registry, &ctx)
//!     .await?;
//! assert!(outcome.stopped_reason.is_success());
//! # Ok(())
//! # }
//! ```

#![forbid(unsafe_code)]

pub mod agent_loop;
pub mod context;
pub mod injection;
pub mod loop_hooks;
pub mod message;
pub mod model;
pub mod orchestrator;
pub mod profile;
pub mod provider;
pub mod react;
pub mod scheduler;
pub mod session;
pub mod session_store;
pub mod skills;
pub mod subagent;

// ---- Messages & session ----
pub use message::{Message, Role, ToolCallRequest, ToolResultRef};
pub use session::Session;
pub use session_store::{SessionRecord, SessionStore, SessionSummary};

// ---- Context management & compression ----
pub use context::{
    estimate_message_tokens, estimate_tokens, ContextManager, HeuristicSummarizer, Summarizer,
};

// ---- Model provider & orchestration ----
pub use model::{
    CompletionRequest, CompletionResponse, FinishReason, MockProvider, ModelProvider, Protocol,
    SamplingParams,
};
pub use orchestrator::{AgentAction, Orchestrator};

// ---- Real LLM providers + multi-provider/model configuration ----
pub use provider::{
    test_connection, HttpModelProvider, ProviderConfig, ProviderProtocol, ProviderSettings,
    ProviderStore, ProviderToolMode,
};

// ---- ReAct protocol ----
pub use react::{parse_react, render_react_system, ReActStep};

// ---- Prompt-injection guard ----
pub use injection::{InjectionHit, PromptInjectionGuard, Severity};

// ---- Scheduling ----
pub use scheduler::ToolScheduler;

// ---- The agent loop ----
pub use agent_loop::{loop_guard_error, GoalLoop, LoopGuard, LoopOutcome, StoppedReason};

// ---- Loop / model observability hooks ----
pub use loop_hooks::{LoopEvent, LoopHook, LoopHookRegistry, RecordingLoopHook};

// ---- Reusable skills (playbooks) ----
pub use skills::{skill_system_message, Skill, SkillListTool, SkillLoadTool, SkillRegistry};

// ---- Subagents (bounded delegated runs) ----
pub use subagent::{SubagentTool, DEFAULT_SUBAGENT_MAX_STEPS};

// ---- Project profile (writer.md / outline.md steering) ----
pub use profile::ProjectProfile;

// ---- Story state management (consistency enhancement) ----
pub use na_story::{
    CharacterState, ConsistencyGuard, ConsistencyReport, Constraint, ContextPackage,
    ForeshadowTracker, KnowledgeMatrix, Severity as StorySeverity, StoryState,
    StoryStateManager, render_state_sync_prompt,
};

// ---- Convenience common re-exports most callers of this crate will need ----
pub use na_common::{CancellationToken, CoreError, Json, Result, SessionId};

use std::sync::Arc;

/// Register the runtime-level tools — [`SkillListTool`], [`SkillLoadTool`], and
/// [`SubagentTool`] — into an existing [`ToolRegistry`](na_tools::ToolRegistry).
///
/// These three tools need runtime collaborators the built-in tool layer has no
/// knowledge of: the skill tools are backed by a shared [`SkillRegistry`], and
/// the subagent tool needs a [`ModelProvider`] plus the very registry it will
/// run children against. Wire them in with one call after building the base
/// registry:
///
/// ```no_run
/// use std::sync::Arc;
/// use na_runtime::{register_runtime_tools, MockProvider, SkillRegistry, CompletionResponse};
/// use na_tools::builtin_registry;
///
/// let provider: Arc<dyn na_runtime::ModelProvider> =
///     Arc::new(MockProvider::from_responses(vec![CompletionResponse::answer("ok")]));
/// let skills = Arc::new(SkillRegistry::new());
/// let mut registry = builtin_registry();
/// register_runtime_tools(&mut registry, provider, skills);
/// assert!(registry.contains("skill_list"));
/// assert!(registry.contains("skill_load"));
/// assert!(registry.contains("spawn_subagent"));
/// ```
///
/// The `SubagentTool` is given an `Arc` snapshot of the registry *as it exists
/// after the skill tools are added* (but before the subagent tool itself), so a
/// spawned child can use every base + skill tool. Existing tools with the same
/// names are replaced, so the call is idempotent.
pub fn register_runtime_tools(
    registry: &mut na_tools::ToolRegistry,
    provider: Arc<dyn ModelProvider>,
    skills: Arc<SkillRegistry>,
) {
    // Skill tools first.
    registry.register_or_replace(Arc::new(SkillListTool::new(skills.clone())));
    registry.register_or_replace(Arc::new(SkillLoadTool::new(skills)));

    // Snapshot the registry (base + skill tools) for the subagent's children,
    // then add the subagent tool. The snapshot intentionally excludes the
    // subagent tool itself to avoid a child trivially re-spawning grandchildren
    // through the same shared `Arc` (children get a focused, finite toolset).
    let child_registry = Arc::new(registry.clone());
    registry.register_or_replace(Arc::new(SubagentTool::new(provider, child_registry)));
}

/// A boxed, `Send` future with an explicit lifetime — the object-safe stand-in
/// for `async fn` in this crate's traits. Re-exported at the crate root for
/// downstream implementors of [`ModelProvider`] / [`Summarizer`](context::Summarizer).
pub type BoxFuture<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

#[cfg(test)]
mod tests {
    //! Crate-level smoke tests exercising the public surface together.

    use super::*;
    use na_common::json;
    use na_tools::{builtin_registry, ToolContextBuilder};

    fn temp_root(tag: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "na_runtime_lib_{}_{}",
            tag,
            na_common::next_id("t")
        ));
        p
    }

    #[test]
    fn traits_are_object_safe() {
        fn assert_obj<T: ?Sized>() {}
        assert_obj::<dyn ModelProvider>();
        assert_obj::<dyn Summarizer>();
    }

    #[tokio::test]
    async fn end_to_end_goal_loop_writes_a_file() {
        // A full, offline agent run: the model asks to write a chapter, then
        // declares the goal complete. We verify the file was actually written by
        // the tool layer and the loop reports success.
        let provider = MockProvider::from_responses(vec![
            CompletionResponse::tool_call(ToolCallRequest::new(
                "write_file",
                json!({ "path": "book/ch1.md", "content": "第一章\n林惊羽提剑而立。" }),
            )),
            CompletionResponse::answer("第一章已经写好，林惊羽登场。"),
        ]);
        let registry = builtin_registry();
        let ctx = ToolContextBuilder::new(temp_root("e2e")).build().unwrap();
        let mut session = Session::new("修仙长篇");

        let outcome = GoalLoop::with_protocol(Protocol::NativeToolCall)
            .max_steps(8)
            .run("写第一章", &mut session, &provider, &registry, &ctx)
            .await
            .unwrap();

        assert_eq!(outcome.stopped_reason, StoppedReason::GoalReached);
        assert!(outcome.final_answer.unwrap().contains("林惊羽"));

        // The file exists on disk via the real write_file tool.
        let read = registry
            .invoke("read_file", json!({ "path": "book/ch1.md" }), &ctx)
            .await;
        assert!(read.ok);
        assert!(read.content.contains("林惊羽提剑而立"));

        // Session can be persisted and reloaded losslessly.
        let path = ctx.jail.root().join("session.json");
        session.save(&path).unwrap();
        let reloaded = Session::load(&path).unwrap();
        assert_eq!(session, reloaded);
    }

    #[test]
    fn reexports_compile_and_construct() {
        // Touch the re-exported types so the public surface is exercised.
        let _g = PromptInjectionGuard::default();
        let _cm = ContextManager::default();
        let _s = ToolScheduler::new();
        let _o = Orchestrator::default();
        let _loop = GoalLoop::default();
        let _msg = Message::system("x");
        let _skills = SkillRegistry::new();
        let _profile = ProjectProfile::default();
        let _hooks = LoopHookRegistry::new();
        assert_eq!(estimate_tokens("你好"), 2);
    }

    #[tokio::test]
    async fn register_runtime_tools_wires_skills_and_subagent() {
        use std::sync::Arc;

        // A skill registry with one playbook the skill tools can surface.
        let mut sreg = SkillRegistry::new();
        sreg.register(skills::Skill::new(
            "outline",
            "Outline a story arc.",
            "Write three acts.",
            vec!["write_file".to_string()],
        ));
        let sreg = Arc::new(sreg);

        // The subagent's scripted child writes a file then finishes.
        let provider: Arc<dyn ModelProvider> = Arc::new(MockProvider::from_responses(vec![
            CompletionResponse::tool_call(ToolCallRequest::new(
                "write_file",
                json!({ "path": "sub.md", "content": "子代理写的内容" }),
            )),
            CompletionResponse::answer("子任务完成。"),
        ]));

        let mut registry = builtin_registry();
        let base = registry.len();
        register_runtime_tools(&mut registry, provider, sreg);

        // Three runtime tools were added.
        assert_eq!(registry.len(), base + 3);
        assert!(registry.contains("skill_list"));
        assert!(registry.contains("skill_load"));
        assert!(registry.contains("spawn_subagent"));

        let ctx = ToolContextBuilder::new(temp_root("regtools"))
            .build()
            .unwrap();

        // skill_load returns the playbook body.
        let loaded = registry
            .invoke("skill_load", json!({ "name": "outline" }), &ctx)
            .await;
        assert!(loaded.ok);
        assert_eq!(loaded.content, "Write three acts.");

        // spawn_subagent runs a bounded child, writes the file, returns a summary.
        let spawned = registry
            .invoke(
                "spawn_subagent",
                json!({ "goal": "写一个文件", "title": "子任务" }),
                &ctx,
            )
            .await;
        assert!(spawned.ok);
        assert_eq!(spawned.data["success"], true);
        assert_eq!(spawned.data["stopped_reason"], "goal_reached");

        // The child's side effect landed on disk.
        let read = registry
            .invoke("read_file", json!({ "path": "sub.md" }), &ctx)
            .await;
        assert!(read.ok);
        assert!(read.content.contains("子代理写的内容"));
    }
}
