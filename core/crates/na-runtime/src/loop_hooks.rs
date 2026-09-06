//! Observability hooks for the [`GoalLoop`](crate::agent_loop::GoalLoop).
//!
//! A [`LoopHook`] is a passive observer fired at the three meaningful points of
//! a run — step/model progress, each tool lifecycle, and the final outcome.
//! Hooks are for *observability and instrumentation* (logging, metrics,
//! progress UIs); they do not alter control flow. A [`GoalLoop`] with an empty
//! [`LoopHookRegistry`] (the default) behaves byte-for-byte as before.
//!
//! Hooks must be cheap and must not block — they run synchronously on the loop
//! task. The bundled [`RecordingLoopHook`] captures every event into a shared
//! vector for tests and simple in-process dashboards.

use std::sync::{Arc, Mutex};

use crate::agent_loop::LoopOutcome;
use crate::message::ToolCallRequest;
use crate::model::CompletionResponse;
use crate::session::Session;
use na_tools::ToolResult;

/// A deliberately data-minimal tool outcome for progress UIs and telemetry.
///
/// Full tool output, arguments, paths, commands, and provider error messages are
/// intentionally excluded. `summary` is generated only from non-content result
/// metadata; `error` is a short, validated error code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolExecutionOutcome {
    /// Whether the tool completed successfully.
    pub ok: bool,
    /// Tool-reported execution duration in milliseconds.
    pub duration_ms: u64,
    /// Safe categorical success summary.
    pub summary: Option<String>,
    /// Safe error code when `ok` is false.
    pub error: Option<String>,
}

impl ToolExecutionOutcome {
    /// Derive a safe lifecycle payload without copying tool content or data.
    pub fn from_result(result: &ToolResult) -> Self {
        if result.ok {
            let mut details = Vec::new();
            if result.metadata.truncated {
                details.push("truncated");
            }
            if result.metadata.redactions > 0 {
                details.push("redacted");
            }
            if result.metadata.was_binary {
                details.push("binary");
            }
            if result.metadata.untrusted {
                details.push("untrusted");
            }
            let summary = if details.is_empty() {
                "completed".to_string()
            } else {
                format!("completed ({})", details.join(", "))
            };
            ToolExecutionOutcome {
                ok: true,
                duration_ms: result.metadata.duration_ms,
                summary: Some(summary),
                error: None,
            }
        } else {
            let code = result
                .data
                .get("code")
                .and_then(|value| value.as_str())
                .map(safe_error_code)
                .unwrap_or_else(|| "tool_failed".to_string());
            ToolExecutionOutcome {
                ok: false,
                duration_ms: result.metadata.duration_ms,
                summary: None,
                error: Some(code),
            }
        }
    }

    /// Construct a synthetic failure for a scheduler-level interruption.
    pub fn interrupted(duration_ms: u64, error: &str) -> Self {
        ToolExecutionOutcome {
            ok: false,
            duration_ms,
            summary: None,
            error: Some(safe_error_code(error)),
        }
    }
}

fn safe_error_code(value: &str) -> String {
    if !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        value.to_string()
    } else {
        "tool_failed".to_string()
    }
}

/// A passive observer of a [`GoalLoop`](crate::agent_loop::GoalLoop) run.
///
/// All methods default to no-ops, so an implementor overrides only the events it
/// cares about. The trait is `Send + Sync` so hooks can be held as
/// `Arc<dyn LoopHook>` and shared across tasks.
pub trait LoopHook: Send + Sync {
    /// A short hook name (for logging / debugging).
    fn name(&self) -> &str {
        "loop_hook"
    }

    /// Fired at the top of each iteration, *after* the per-step guards pass and
    /// the step counter has been incremented. `step` is 1-based.
    fn on_step_start(&self, step: u32, session: &Session) {
        let _ = (step, session);
    }

    /// Fired for each streamed text fragment as the model writes step `step`'s
    /// answer (token by token). Providers without streaming emit the whole text
    /// as a single delta; either way [`on_model_response`](Self::on_model_response)
    /// still fires once with the complete response.
    fn on_model_delta(&self, step: u32, delta: &str) {
        let _ = (step, delta);
    }

    /// Fired immediately after the model returns a completion for `step`.
    fn on_model_response(&self, step: u32, resp: &CompletionResponse) {
        let _ = (step, resp);
    }

    /// Fired immediately before a stable tool-call id is dispatched.
    fn on_tool_start(&self, step: u32, call: &ToolCallRequest) {
        let _ = (step, call);
    }

    /// Fired once after that tool call finishes or is interrupted.
    fn on_tool_finish(&self, step: u32, call: &ToolCallRequest, outcome: &ToolExecutionOutcome) {
        let _ = (step, call, outcome);
    }

    /// Fired exactly once, just before the loop returns, with the final outcome.
    fn on_finish(&self, outcome: &LoopOutcome) {
        let _ = outcome;
    }
}

/// An ordered collection of [`LoopHook`]s with fan-out fire helpers.
#[derive(Clone, Default)]
pub struct LoopHookRegistry {
    hooks: Vec<Arc<dyn LoopHook>>,
}

impl std::fmt::Debug for LoopHookRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoopHookRegistry")
            .field("hooks", &self.names())
            .finish()
    }
}

impl LoopHookRegistry {
    /// An empty registry (fires nothing).
    pub fn new() -> Self {
        LoopHookRegistry { hooks: Vec::new() }
    }

    /// Register a hook (fired in registration order).
    pub fn register(&mut self, hook: Arc<dyn LoopHook>) {
        self.hooks.push(hook);
    }

    /// Number of registered hooks.
    pub fn len(&self) -> usize {
        self.hooks.len()
    }

    /// Whether the registry has no hooks.
    pub fn is_empty(&self) -> bool {
        self.hooks.is_empty()
    }

    /// The names of all registered hooks.
    pub fn names(&self) -> Vec<&str> {
        self.hooks.iter().map(|h| h.name()).collect()
    }

    /// Fire [`LoopHook::on_step_start`] on every hook.
    pub fn fire_step_start(&self, step: u32, session: &Session) {
        for h in &self.hooks {
            h.on_step_start(step, session);
        }
    }

    /// Fire [`LoopHook::on_model_delta`] on every hook.
    pub fn fire_model_delta(&self, step: u32, delta: &str) {
        for h in &self.hooks {
            h.on_model_delta(step, delta);
        }
    }

    /// Fire [`LoopHook::on_model_response`] on every hook.
    pub fn fire_model_response(&self, step: u32, resp: &CompletionResponse) {
        for h in &self.hooks {
            h.on_model_response(step, resp);
        }
    }

    /// Fire [`LoopHook::on_tool_start`] on every hook.
    pub fn fire_tool_start(&self, step: u32, call: &ToolCallRequest) {
        for h in &self.hooks {
            h.on_tool_start(step, call);
        }
    }

    /// Fire [`LoopHook::on_tool_finish`] on every hook.
    pub fn fire_tool_finish(
        &self,
        step: u32,
        call: &ToolCallRequest,
        outcome: &ToolExecutionOutcome,
    ) {
        for h in &self.hooks {
            h.on_tool_finish(step, call, outcome);
        }
    }

    /// Fire [`LoopHook::on_finish`] on every hook.
    pub fn fire_finish(&self, outcome: &LoopOutcome) {
        for h in &self.hooks {
            h.on_finish(outcome);
        }
    }
}

/// One observed loop event, captured by [`RecordingLoopHook`].
#[derive(Debug, Clone, PartialEq)]
pub enum LoopEvent {
    /// A step began (1-based step number + session length at that moment).
    StepStart { step: u32, history_len: usize },
    /// The model answered at `step` (with its finish reason and tool-call count).
    ModelResponse {
        step: u32,
        finish: crate::model::FinishReason,
        tool_calls: usize,
    },
    /// A tool call was dispatched using its stable correlation id.
    ToolStart { step: u32, id: String, name: String },
    /// A tool call reached a terminal result.
    ToolFinish {
        step: u32,
        id: String,
        name: String,
        outcome: ToolExecutionOutcome,
    },
    /// The loop finished with this outcome.
    Finish { outcome: LoopOutcome },
}

/// A [`LoopHook`] that records every event into a shared, lock-protected vector.
///
/// Clone it freely — clones share the same underlying log (an
/// `Arc<Mutex<Vec<_>>>`), so a copy handed to the loop and a copy kept by the
/// caller observe the same events. Useful for tests and lightweight progress
/// displays.
#[derive(Clone)]
pub struct RecordingLoopHook {
    name: String,
    events: Arc<Mutex<Vec<LoopEvent>>>,
}

impl std::fmt::Debug for RecordingLoopHook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecordingLoopHook")
            .field("name", &self.name)
            .field("events", &self.len())
            .finish()
    }
}

impl Default for RecordingLoopHook {
    fn default() -> Self {
        Self::new()
    }
}

impl RecordingLoopHook {
    /// A fresh recorder with an empty log.
    pub fn new() -> Self {
        RecordingLoopHook {
            name: "recording".to_string(),
            events: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// A recorder with a custom name.
    pub fn named(name: impl Into<String>) -> Self {
        RecordingLoopHook {
            name: name.into(),
            events: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// A snapshot of the events recorded so far.
    pub fn events(&self) -> Vec<LoopEvent> {
        self.events.lock().map(|v| v.clone()).unwrap_or_default()
    }

    /// How many events have been recorded.
    pub fn len(&self) -> usize {
        self.events.lock().map(|v| v.len()).unwrap_or(0)
    }

    /// Whether nothing has been recorded yet.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Count the recorded [`LoopEvent::StepStart`] events.
    pub fn step_count(&self) -> usize {
        self.events()
            .iter()
            .filter(|e| matches!(e, LoopEvent::StepStart { .. }))
            .count()
    }

    /// The recorded finish outcome, if the loop has finished.
    pub fn finish_outcome(&self) -> Option<LoopOutcome> {
        self.events().into_iter().find_map(|e| match e {
            LoopEvent::Finish { outcome } => Some(outcome),
            _ => None,
        })
    }

    fn push(&self, event: LoopEvent) {
        if let Ok(mut v) = self.events.lock() {
            v.push(event);
        }
    }
}

impl LoopHook for RecordingLoopHook {
    fn name(&self) -> &str {
        &self.name
    }

    fn on_step_start(&self, step: u32, session: &Session) {
        self.push(LoopEvent::StepStart {
            step,
            history_len: session.len(),
        });
    }

    fn on_model_response(&self, step: u32, resp: &CompletionResponse) {
        self.push(LoopEvent::ModelResponse {
            step,
            finish: resp.finish,
            tool_calls: resp.tool_calls.len(),
        });
    }

    fn on_tool_start(&self, step: u32, call: &ToolCallRequest) {
        self.push(LoopEvent::ToolStart {
            step,
            id: call.id.to_string(),
            name: call.name.clone(),
        });
    }

    fn on_tool_finish(&self, step: u32, call: &ToolCallRequest, outcome: &ToolExecutionOutcome) {
        self.push(LoopEvent::ToolFinish {
            step,
            id: call.id.to_string(),
            name: call.name.clone(),
            outcome: outcome.clone(),
        });
    }

    fn on_finish(&self, outcome: &LoopOutcome) {
        self.push(LoopEvent::Finish {
            outcome: outcome.clone(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_loop::StoppedReason;
    use crate::model::FinishReason;
    use na_common::json;

    #[test]
    fn empty_registry_fires_nothing() {
        let reg = LoopHookRegistry::new();
        assert!(reg.is_empty());
        assert_eq!(reg.len(), 0);
        let session = Session::new("t");
        let call = ToolCallRequest::new("note", json!({ "private": "not emitted" }));
        let tool_outcome = ToolExecutionOutcome::from_result(&ToolResult::success(
            "private result",
            json!({ "private": "not emitted" }),
        ));
        // These must be safe no-ops on an empty registry.
        reg.fire_step_start(1, &session);
        reg.fire_model_response(1, &CompletionResponse::answer("x"));
        reg.fire_tool_start(1, &call);
        reg.fire_tool_finish(1, &call, &tool_outcome);
        reg.fire_finish(&LoopOutcome {
            stopped_reason: StoppedReason::GoalReached,
            steps: 1,
            final_answer: Some("x".into()),
        });
    }

    #[test]
    fn recording_hook_collects_events_in_order() {
        let hook = RecordingLoopHook::new();
        let mut reg = LoopHookRegistry::new();
        reg.register(Arc::new(hook.clone()));
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.names(), vec!["recording"]);

        let mut session = Session::new("t");
        session.push(crate::message::Message::user("hi"));

        reg.fire_step_start(1, &session);
        reg.fire_model_response(1, &CompletionResponse::answer("done"));
        let call = ToolCallRequest::with_id(
            na_common::ToolCallId::from_existing("call_recorded"),
            "note",
            json!({ "text": "private" }),
        );
        let tool_outcome = ToolExecutionOutcome::from_result(&ToolResult::success(
            "private result",
            json!({ "secret": "private" }),
        ));
        reg.fire_tool_start(1, &call);
        reg.fire_tool_finish(1, &call, &tool_outcome);
        let outcome = LoopOutcome {
            stopped_reason: StoppedReason::GoalReached,
            steps: 1,
            final_answer: Some("done".into()),
        };
        reg.fire_finish(&outcome);

        let events = hook.events();
        assert_eq!(events.len(), 5);
        assert_eq!(
            events[0],
            LoopEvent::StepStart {
                step: 1,
                history_len: 1
            }
        );
        assert_eq!(
            events[1],
            LoopEvent::ModelResponse {
                step: 1,
                finish: FinishReason::Stop,
                tool_calls: 0
            }
        );
        assert_eq!(
            events[2],
            LoopEvent::ToolStart {
                step: 1,
                id: "call_recorded".to_string(),
                name: "note".to_string(),
            }
        );
        assert_eq!(
            events[3],
            LoopEvent::ToolFinish {
                step: 1,
                id: "call_recorded".to_string(),
                name: "note".to_string(),
                outcome: tool_outcome,
            }
        );
        assert_eq!(events[4], LoopEvent::Finish { outcome });
        assert_eq!(hook.step_count(), 1);
        assert_eq!(
            hook.finish_outcome().unwrap().stopped_reason,
            StoppedReason::GoalReached
        );
    }

    #[test]
    fn clones_share_the_same_log() {
        let hook = RecordingLoopHook::named("shared");
        let clone = hook.clone();
        let session = Session::new("t");
        hook.on_step_start(1, &session);
        // The clone observes the event recorded through the original.
        assert_eq!(clone.len(), 1);
        assert_eq!(clone.name(), "shared");
    }

    #[test]
    fn fan_out_to_multiple_hooks() {
        let a = RecordingLoopHook::named("a");
        let b = RecordingLoopHook::named("b");
        let mut reg = LoopHookRegistry::new();
        reg.register(Arc::new(a.clone()));
        reg.register(Arc::new(b.clone()));
        let session = Session::new("t");
        reg.fire_step_start(7, &session);
        assert_eq!(a.step_count(), 1);
        assert_eq!(b.step_count(), 1);
    }

    #[test]
    fn tool_outcome_never_copies_content_or_untrusted_error_text() {
        let success = ToolResult::success(
            "token=super-secret",
            json!({ "command": "private command" }),
        )
        .with_summary("private path and command");
        let safe = ToolExecutionOutcome::from_result(&success);
        assert_eq!(safe.summary.as_deref(), Some("completed"));
        assert_eq!(safe.error, None);
        assert!(!format!("{safe:?}").contains("secret"));
        assert!(!format!("{safe:?}").contains("private"));

        let mut failure = ToolResult::from_error(&na_common::CoreError::tool(
            "provider returned private credentials",
        ));
        failure.data["code"] = json!("unsafe code with spaces");
        let safe = ToolExecutionOutcome::from_result(&failure);
        assert_eq!(safe.summary, None);
        assert_eq!(safe.error.as_deref(), Some("tool_failed"));
        assert!(!format!("{safe:?}").contains("credentials"));
    }
}
