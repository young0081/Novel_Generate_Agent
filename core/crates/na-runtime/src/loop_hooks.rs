//! Observability hooks for the [`GoalLoop`](crate::agent_loop::GoalLoop).
//!
//! A [`LoopHook`] is a passive observer fired at the three meaningful points of
//! a run — the start of each step, after the model answers, and once when the
//! loop finishes. Hooks are for *observability and instrumentation* (logging,
//! metrics, progress UIs); they do not alter control flow. A [`GoalLoop`] with
//! an empty [`LoopHookRegistry`] (the default) behaves byte-for-byte as before.
//!
//! Hooks must be cheap and must not block — they run synchronously on the loop
//! task. The bundled [`RecordingLoopHook`] captures every event into a shared
//! vector for tests and simple in-process dashboards.

use std::sync::{Arc, Mutex};

use crate::agent_loop::LoopOutcome;
use crate::model::CompletionResponse;
use crate::session::Session;

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

    #[test]
    fn empty_registry_fires_nothing() {
        let reg = LoopHookRegistry::new();
        assert!(reg.is_empty());
        assert_eq!(reg.len(), 0);
        let session = Session::new("t");
        // These must be safe no-ops on an empty registry.
        reg.fire_step_start(1, &session);
        reg.fire_model_response(1, &CompletionResponse::answer("x"));
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
        let outcome = LoopOutcome {
            stopped_reason: StoppedReason::GoalReached,
            steps: 1,
            final_answer: Some("done".into()),
        };
        reg.fire_finish(&outcome);

        let events = hook.events();
        assert_eq!(events.len(), 3);
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
        assert_eq!(events[2], LoopEvent::Finish { outcome });
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
}
