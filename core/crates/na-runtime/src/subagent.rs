//! The **subagent** tool — delegate a sub-goal to a bounded child agent and get
//! back only a concise summary.
//!
//! [`SubagentTool`] (`spawn_subagent`) lets a parent agent fan out a focused
//! sub-task to a fresh child [`GoalLoop`] that runs against its own
//! [`Session`](crate::session::Session). The whole point is **context economy**:
//! the child may take many internal steps and accumulate a long transcript, but
//! the parent only ever sees a short structured summary — the final answer, why
//! the child stopped, how many steps it took, and a brief tail of what it did.
//! The child's verbose history never pollutes the parent's context window.
//!
//! Because a [`Tool::execute`] only receives `(args, &ToolContext)` yet a
//! subagent needs a [`ModelProvider`] and a [`ToolRegistry`] to run, the tool
//! holds those (as `Arc`s) in its own struct. The child reuses the parent's
//! [`ToolContext`] (same jail, policy, stores) but with a **child cancellation
//! token** ([`CancellationToken::child`](na_common::CancellationToken::child)),
//! so cancelling the parent cancels the child, and a reduced step budget.

use std::sync::Arc;

use na_common::{json, CoreError, Json, Result};
use na_tools::{BoxFuture, Tool, ToolContext, ToolRegistry, ToolResult, ToolSpec};

use crate::agent_loop::GoalLoop;
use crate::model::{ModelProvider, Protocol};
use crate::session::Session;

/// The default step budget for a spawned subagent when the caller does not
/// override it via the `max_steps` argument. Deliberately small to keep child
/// runs cheap and bounded.
pub const DEFAULT_SUBAGENT_MAX_STEPS: u32 = 6;

/// A hard ceiling on the requested `max_steps` so a runaway argument cannot ask
/// for an unbounded child run.
const SUBAGENT_MAX_STEPS_CEILING: u32 = 64;

/// How many trailing transcript lines to include in the summary tail.
const SUMMARY_TAIL_STEPS: usize = 4;

/// A tool that runs a bounded child agent toward a sub-goal and returns a
/// concise summary of the result (never the full child transcript).
#[derive(Clone)]
pub struct SubagentTool {
    provider: Arc<dyn ModelProvider>,
    registry: Arc<ToolRegistry>,
    /// Default step budget for spawned children (overridable per call).
    pub max_steps: u32,
    /// Protocol the child loop uses.
    pub protocol: Protocol,
}

impl std::fmt::Debug for SubagentTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SubagentTool")
            .field("provider", &self.provider.name())
            .field("tools", &self.registry.len())
            .field("max_steps", &self.max_steps)
            .field("protocol", &self.protocol)
            .finish()
    }
}

impl SubagentTool {
    /// Build a subagent tool over a model `provider` and tool `registry`, using
    /// the default step budget and the native-tool-call protocol.
    pub fn new(provider: Arc<dyn ModelProvider>, registry: Arc<ToolRegistry>) -> Self {
        SubagentTool {
            provider,
            registry,
            max_steps: DEFAULT_SUBAGENT_MAX_STEPS,
            protocol: Protocol::NativeToolCall,
        }
    }

    /// Builder: set the default step budget for children.
    pub fn max_steps(mut self, n: u32) -> Self {
        self.max_steps = n.clamp(1, SUBAGENT_MAX_STEPS_CEILING);
        self
    }

    /// Builder: set the protocol the child loop runs in.
    pub fn protocol(mut self, protocol: Protocol) -> Self {
        self.protocol = protocol;
        self
    }

    /// Resolve the effective step budget from an optional override argument.
    fn effective_steps(&self, requested: Option<u32>) -> u32 {
        requested
            .unwrap_or(self.max_steps)
            .clamp(1, SUBAGENT_MAX_STEPS_CEILING)
    }
}

/// Build a short, human-readable tail describing the last few things the child
/// did, derived from its session transcript (NOT the full history).
fn summarize_tail(session: &Session) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for msg in session.history() {
        // Surface tool calls and tool observations — the "actions" — plus any
        // non-empty assistant narration. Skip the seeded user goal + system.
        match msg.role {
            crate::message::Role::Assistant => {
                if let Some(call) = &msg.tool_call {
                    lines.push(format!("→ called {}", call.name));
                } else if !msg.content.trim().is_empty() {
                    lines.push(format!("• {}", first_line_clipped(&msg.content, 120)));
                }
            }
            crate::message::Role::Tool => {
                if let Some(r) = &msg.tool_result {
                    let status = if r.ok { "ok" } else { "error" };
                    lines.push(format!("← {} [{}]", r.name, status));
                }
            }
            _ => {}
        }
    }
    // Keep only the tail.
    let start = lines.len().saturating_sub(SUMMARY_TAIL_STEPS);
    lines.split_off(start)
}

/// The first line of `s`, clipped to at most `max` characters (char-safe).
fn first_line_clipped(s: &str, max: usize) -> String {
    let first = s.lines().next().unwrap_or("").trim();
    if first.chars().count() <= max {
        first.to_string()
    } else {
        let clipped: String = first.chars().take(max).collect();
        format!("{clipped}…")
    }
}

impl Tool for SubagentTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "spawn_subagent",
            "Delegate a focused sub-task to a bounded child agent and get back a \
             concise summary (final answer + stop reason + step count + a short \
             tail of what it did) — NOT the full child transcript. Use this to \
             keep your own context small while a sub-task runs to completion.",
            json!({
                "type": "object",
                "required": ["goal"],
                "properties": {
                    "goal": { "type": "string", "minLength": 1 },
                    "title": { "type": "string" },
                    "max_steps": { "type": "integer", "minimum": 1, "maximum": 64 }
                },
                "additionalProperties": false
            }),
            // No special capability: the child reuses the parent context's perms,
            // and every tool it calls is itself authorized through that context.
            vec![],
            true,
        )
        .with_subagent_concurrency()
    }

    fn execute<'a>(
        &'a self,
        args: Json,
        ctx: &'a ToolContext,
    ) -> BoxFuture<'a, Result<ToolResult>> {
        Box::pin(async move {
            let goal = args
                .get("goal")
                .and_then(Json::as_str)
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| CoreError::invalid_input("missing string argument \"goal\""))?
                .to_string();
            let title = args
                .get("title")
                .and_then(Json::as_str)
                .unwrap_or("subagent")
                .to_string();
            let requested = args
                .get("max_steps")
                .and_then(Json::as_u64)
                .map(|n| n as u32);
            let max_steps = self.effective_steps(requested);

            // Honor cancellation up front.
            ctx.cancel.check()?;

            // The child runs under a CHILD cancellation token: cancelling the
            // parent cancels the child, but the child cannot cancel the parent.
            let mut child_ctx = ctx.clone();
            child_ctx.cancel = ctx.cancel.child();

            let mut child_session = Session::new(title.clone());

            let child_loop = GoalLoop::with_protocol(self.protocol).max_steps(max_steps);
            let outcome = child_loop
                .run(
                    &goal,
                    &mut child_session,
                    self.provider.as_ref(),
                    self.registry.as_ref(),
                    &child_ctx,
                )
                .await?;

            // Build the CONCISE summary — this is the only thing the parent sees.
            let tail = summarize_tail(&child_session);
            let answer = outcome.final_answer.clone().unwrap_or_default();

            let mut content = format!(
                "subagent {title:?} finished: {} after {} step(s).",
                outcome.stopped_reason.as_str(),
                outcome.steps
            );
            if !answer.trim().is_empty() {
                content.push_str(&format!("\nresult: {answer}"));
            }
            if !tail.is_empty() {
                content.push_str("\nrecent:\n");
                content.push_str(
                    &tail
                        .iter()
                        .map(|l| format!("  {l}"))
                        .collect::<Vec<_>>()
                        .join("\n"),
                );
            }

            let data = json!({
                "title": title,
                "final_answer": answer,
                "stopped_reason": outcome.stopped_reason.as_str(),
                "success": outcome.stopped_reason.is_success(),
                "steps": outcome.steps,
                "tail": tail,
            });

            Ok(ToolResult::success(content, data).with_summary(format!(
                "subagent {}: {}",
                title,
                outcome.stopped_reason.as_str()
            )))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::ToolCallRequest;
    use crate::model::{CompletionResponse, MockProvider};
    use na_common::CancellationToken;
    use na_tools::{builtin_registry, ToolContextBuilder};

    fn temp_root(tag: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "na_runtime_subagent_{}_{}",
            tag,
            na_common::next_id("t")
        ));
        p
    }

    #[tokio::test]
    async fn subagent_writes_a_file_and_returns_summary() {
        // The child writes a chapter, then declares success.
        let provider = Arc::new(MockProvider::from_responses(vec![
            CompletionResponse::tool_call(ToolCallRequest::new(
                "write_file",
                json!({ "path": "sub/ch1.md", "content": "第一章\n少年提剑。" }),
            )),
            CompletionResponse::answer("子任务完成：已写好第一章。"),
        ]));
        let registry = Arc::new(builtin_registry());
        let tool = SubagentTool::new(provider, registry.clone());

        let ctx = ToolContextBuilder::new(temp_root("write")).build().unwrap();
        let res = tool
            .execute(json!({ "goal": "写第一章", "title": "第一章子任务" }), &ctx)
            .await
            .unwrap();

        assert!(res.ok);
        // The structured result is a SUMMARY.
        assert_eq!(res.data["success"], true);
        assert_eq!(res.data["stopped_reason"], "goal_reached");
        assert!(res.data["steps"].as_u64().unwrap() >= 2);
        assert_eq!(res.data["final_answer"], "子任务完成：已写好第一章。");

        // The side effect actually happened: the file is on disk.
        let read = registry
            .invoke("read_file", json!({ "path": "sub/ch1.md" }), &ctx)
            .await;
        assert!(read.ok);
        assert!(read.content.contains("少年提剑"));

        // Crucially, the returned content is a concise summary, NOT the entire
        // child transcript: it must not contain the raw chapter prose body.
        assert!(res.content.contains("subagent"));
        assert!(res.content.contains("goal_reached"));
        assert!(
            !res.content.contains("少年提剑。"),
            "summary must not embed the full child transcript/prose"
        );
        // The tail mentions the actions, not the full content.
        let tail = res.data["tail"].as_array().unwrap();
        assert!(tail
            .iter()
            .any(|l| l.as_str().unwrap().contains("write_file")));
    }

    #[tokio::test]
    async fn subagent_via_registry_lifecycle() {
        let provider = Arc::new(MockProvider::from_responses(vec![
            CompletionResponse::answer("nothing to do"),
        ]));
        let registry = Arc::new(builtin_registry());
        let tool = SubagentTool::new(provider, registry);

        let mut reg = ToolRegistry::new();
        reg.register(Arc::new(tool)).unwrap();
        let ctx = ToolContextBuilder::new(temp_root("lifecycle"))
            .build()
            .unwrap();

        let res = reg
            .invoke("spawn_subagent", json!({ "goal": "do something" }), &ctx)
            .await;
        assert!(res.ok);
        assert_eq!(res.data["stopped_reason"], "goal_reached");

        // Missing required goal => invalid_input from the schema validator.
        let bad = reg.invoke("spawn_subagent", json!({}), &ctx).await;
        assert!(!bad.ok);
        assert_eq!(bad.data["code"], "invalid_input");
    }

    #[tokio::test]
    async fn subagent_respects_max_steps_override() {
        // A provider that never finishes; with max_steps=2 the child stops on the
        // step cap and the summary reports it.
        let provider = Arc::new(MockProvider::from_fn(|_req| {
            CompletionResponse::tool_call(ToolCallRequest::new(
                "memory_recall",
                json!({ "query": "x" }),
            ))
        }));
        let registry = Arc::new(builtin_registry());
        let tool = SubagentTool::new(provider, registry).max_steps(10);
        let ctx = ToolContextBuilder::new(temp_root("steps")).build().unwrap();

        let res = tool
            .execute(json!({ "goal": "loop", "max_steps": 2 }), &ctx)
            .await
            .unwrap();
        assert!(res.ok);
        assert_eq!(res.data["stopped_reason"], "max_steps");
        assert_eq!(res.data["steps"], 2);
        assert_eq!(res.data["success"], false);
    }

    #[tokio::test]
    async fn subagent_honors_parent_cancellation() {
        // Parent token already cancelled => the tool bails before running.
        let provider = Arc::new(MockProvider::from_responses(vec![
            CompletionResponse::answer("unreached"),
        ]));
        let registry = Arc::new(builtin_registry());
        let tool = SubagentTool::new(provider, registry);

        let cancel = CancellationToken::new();
        cancel.cancel();
        let ctx = ToolContextBuilder::new(temp_root("cancel"))
            .cancel(cancel)
            .build()
            .unwrap();

        let err = tool.execute(json!({ "goal": "go" }), &ctx).await;
        assert!(err.is_err());
        assert!(err.unwrap_err().is(na_common::ErrorKind::Cancelled));
    }

    #[test]
    fn effective_steps_clamps() {
        let provider = Arc::new(MockProvider::from_responses(vec![]));
        let registry = Arc::new(ToolRegistry::new());
        let tool = SubagentTool::new(provider, registry);
        assert_eq!(tool.effective_steps(None), DEFAULT_SUBAGENT_MAX_STEPS);
        assert_eq!(tool.effective_steps(Some(3)), 3);
        assert_eq!(tool.effective_steps(Some(0)), 1); // clamped up
        assert_eq!(tool.effective_steps(Some(9999)), SUBAGENT_MAX_STEPS_CEILING);
    }

    #[test]
    fn first_line_clipped_is_char_safe() {
        let s = "你好世界这是一个很长的中文句子用来测试裁剪";
        let clipped = first_line_clipped(s, 5);
        assert_eq!(clipped.chars().filter(|c| *c != '…').count(), 5);
        assert!(clipped.ends_with('…'));
    }
}
