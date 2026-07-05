//! Request assembly and response parsing — the bridge between the conversation
//! and the [`ModelProvider`].
//!
//! The [`Orchestrator`] does two jobs:
//!
//! 1. **Assemble** a [`CompletionRequest`] from a session: it windows the history
//!    with the [`ContextManager`], lists the registry's tool specs, and — for the
//!    [`Protocol::ReActText`] protocol — prepends a system preamble describing the
//!    ReAct format and the tool catalog (via [`render_react_system`]).
//!
//! 2. **Parse** a [`CompletionResponse`] into a protocol-independent
//!    [`AgentAction`]: either a list of tool calls to run, or a final answer. For
//!    native tool calls it reads the structured `tool_calls`; for ReAct it parses
//!    the response text with [`parse_react`].

use na_common::Result;
use na_tools::{ToolRegistry, ToolSpec};

use crate::context::ContextManager;
use crate::message::{Message, Role, ToolCallRequest};
use crate::model::{CompletionRequest, CompletionResponse, FinishReason, Protocol};
use crate::react::{parse_react, render_react_system, ReActStep};
use crate::session::Session;

/// What the orchestrator decided the model wants to do this turn.
#[derive(Debug, Clone, PartialEq)]
pub enum AgentAction {
    /// Run these tool calls, then continue the loop.
    ToolCalls {
        /// The model's surrounding narration / thought (may be empty).
        thought: String,
        /// The tool calls to execute.
        calls: Vec<ToolCallRequest>,
    },
    /// The model produced a final answer; the goal is (claimed) complete.
    Final {
        /// The user-facing answer.
        answer: String,
    },
}

/// Builds requests and parses responses for a chosen [`Protocol`].
#[derive(Debug, Clone)]
pub struct Orchestrator {
    /// How the model expresses tool use.
    pub protocol: Protocol,
    /// Context windowing / compression policy used to assemble requests.
    pub context: ContextManager,
}

impl Default for Orchestrator {
    fn default() -> Self {
        Orchestrator {
            protocol: Protocol::NativeToolCall,
            context: ContextManager::default(),
        }
    }
}

impl Orchestrator {
    /// Construct an orchestrator for a protocol with a context manager.
    pub fn new(protocol: Protocol, context: ContextManager) -> Self {
        Orchestrator { protocol, context }
    }

    /// Construct an orchestrator with the given protocol and default context.
    pub fn with_protocol(protocol: Protocol) -> Self {
        Orchestrator {
            protocol,
            context: ContextManager::default(),
        }
    }

    /// Assemble a [`CompletionRequest`] from the current session and registry.
    ///
    /// The history is windowed to the context budget (always keeping system
    /// messages). For [`Protocol::ReActText`], a synthetic system message holding
    /// the ReAct instructions + tool catalog is injected at the front (unless the
    /// session already contains an equivalent one), so a text-only model knows the
    /// exact format and which tools exist.
    pub fn build_request(&self, session: &Session, registry: &ToolRegistry) -> CompletionRequest {
        let specs: Vec<ToolSpec> = registry.list_specs();
        let windowed = self.context.window(session.history());

        let messages = match self.protocol {
            Protocol::NativeToolCall => windowed,
            Protocol::ReActText => prepend_react_preamble(windowed, &specs),
        };

        CompletionRequest::new(messages, specs, self.protocol)
    }

    /// Parse a model response into a protocol-independent [`AgentAction`].
    ///
    /// * Native protocol: if the response carries `tool_calls`, they become a
    ///   [`AgentAction::ToolCalls`]; otherwise the `text` is a
    ///   [`AgentAction::Final`].
    /// * ReAct protocol: the response `text` is parsed with [`parse_react`]; a
    ///   malformed block yields a [`CoreError::protocol`](na_common::CoreError).
    ///
    /// A native response that is explicitly [`FinishReason::Stop`] is always
    /// treated as final even if (defensively) it carried stray tool calls.
    pub fn parse_response(&self, response: &CompletionResponse) -> Result<AgentAction> {
        match self.protocol {
            Protocol::NativeToolCall => Ok(self.parse_native(response)),
            Protocol::ReActText => self.parse_react_response(response),
        }
    }

    /// Native-protocol parsing.
    ///
    /// Tool calls are honored unless the model explicitly signalled
    /// [`FinishReason::Stop`], in which case the response is treated as a final
    /// answer (defensive against a provider that returns both).
    fn parse_native(&self, response: &CompletionResponse) -> AgentAction {
        if !response.tool_calls.is_empty() && response.finish != FinishReason::Stop {
            AgentAction::ToolCalls {
                thought: response.text.clone(),
                calls: response.tool_calls.clone(),
            }
        } else {
            AgentAction::Final {
                answer: response.text.clone(),
            }
        }
    }

    /// ReAct-protocol parsing.
    fn parse_react_response(&self, response: &CompletionResponse) -> Result<AgentAction> {
        let step = parse_react(&response.text)?;
        Ok(match step {
            ReActStep::Action {
                thought,
                tool,
                input,
            } => AgentAction::ToolCalls {
                thought: thought.unwrap_or_default(),
                calls: vec![ToolCallRequest::new(tool, input)],
            },
            ReActStep::Final { answer, .. } => AgentAction::Final { answer },
        })
    }
}

/// Prepend a ReAct system preamble to a windowed message list unless one is
/// already present (heuristically detected by a marker in a leading system
/// message).
fn prepend_react_preamble(mut windowed: Vec<Message>, specs: &[ToolSpec]) -> Vec<Message> {
    let already = windowed
        .iter()
        .any(|m| m.role == Role::System && m.content.contains("emitting a strict ReAct block"));
    if already {
        return windowed;
    }
    let preamble = Message::system(render_react_system(specs));
    windowed.insert(0, preamble);
    windowed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::CompletionResponse;
    use na_common::{json, ToolCallId};
    use na_tools::builtin_registry;

    fn session_with_history() -> Session {
        let mut s = Session::new("t");
        s.push(Message::system("你是写作助手。"));
        s.push(Message::user("写第一章"));
        s
    }

    #[test]
    fn build_request_native_windows_and_lists_specs() {
        let orch = Orchestrator::with_protocol(Protocol::NativeToolCall);
        let reg = builtin_registry();
        let session = session_with_history();
        let req = orch.build_request(&session, &reg);
        assert_eq!(req.protocol, Protocol::NativeToolCall);
        assert_eq!(req.tools.len(), reg.len());
        // System message preserved; no ReAct preamble injected for native.
        assert!(req.messages.iter().any(|m| m.is_system()));
        assert!(!req
            .messages
            .iter()
            .any(|m| m.content.contains("strict ReAct block")));
    }

    #[test]
    fn build_request_react_injects_preamble() {
        let orch = Orchestrator::with_protocol(Protocol::ReActText);
        let reg = builtin_registry();
        let session = session_with_history();
        let req = orch.build_request(&session, &reg);
        assert_eq!(req.protocol, Protocol::ReActText);
        // A ReAct preamble system message is now present and references a tool.
        let preamble = req
            .messages
            .iter()
            .find(|m| m.content.contains("strict ReAct block"));
        assert!(preamble.is_some(), "ReAct preamble must be injected");
        assert!(preamble.unwrap().content.contains("read_file"));
        // It is at the front.
        assert!(req.messages[0].content.contains("strict ReAct block"));
    }

    #[test]
    fn build_request_react_does_not_double_inject() {
        let orch = Orchestrator::with_protocol(Protocol::ReActText);
        let reg = builtin_registry();
        let mut session = Session::new("t");
        // Already contains a preamble-like system message.
        session.push(Message::system(render_react_system(&reg.list_specs())));
        session.push(Message::user("go"));
        let req = orch.build_request(&session, &reg);
        let count = req
            .messages
            .iter()
            .filter(|m| m.content.contains("strict ReAct block"))
            .count();
        assert_eq!(count, 1, "must not inject a second preamble");
    }

    #[test]
    fn parse_native_tool_calls() {
        let orch = Orchestrator::with_protocol(Protocol::NativeToolCall);
        let call = ToolCallRequest::with_id(
            ToolCallId::from_existing("c1"),
            "write_file",
            json!({ "path": "a" }),
        );
        let resp = CompletionResponse::tool_call(call.clone());
        let action = orch.parse_response(&resp).unwrap();
        match action {
            AgentAction::ToolCalls { calls, .. } => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].name, "write_file");
            }
            _ => panic!("expected tool calls"),
        }
    }

    #[test]
    fn parse_native_final_answer() {
        let orch = Orchestrator::with_protocol(Protocol::NativeToolCall);
        let resp = CompletionResponse::answer("第一章写完了。");
        let action = orch.parse_response(&resp).unwrap();
        match action {
            AgentAction::Final { answer } => assert_eq!(answer, "第一章写完了。"),
            _ => panic!("expected final"),
        }
    }

    #[test]
    fn parse_native_stop_with_stray_calls_is_final() {
        let orch = Orchestrator::with_protocol(Protocol::NativeToolCall);
        let call = ToolCallRequest::new("x", json!({}));
        let resp = CompletionResponse {
            text: "all done".into(),
            tool_calls: vec![call],
            finish: FinishReason::Stop,
        };
        let action = orch.parse_response(&resp).unwrap();
        assert!(matches!(action, AgentAction::Final { .. }));
    }

    #[test]
    fn parse_react_action() {
        let orch = Orchestrator::with_protocol(Protocol::ReActText);
        let resp = CompletionResponse::react(
            "Thought: read it\nAction: read_file\nAction Input: {\"path\": \"ch1.md\"}",
        );
        let action = orch.parse_response(&resp).unwrap();
        match action {
            AgentAction::ToolCalls { thought, calls } => {
                assert_eq!(thought, "read it");
                assert_eq!(calls[0].name, "read_file");
                assert_eq!(calls[0].args["path"], "ch1.md");
            }
            _ => panic!("expected tool calls"),
        }
    }

    #[test]
    fn parse_react_final() {
        let orch = Orchestrator::with_protocol(Protocol::ReActText);
        let resp = CompletionResponse::react("Thought: done\nFinal Answer: 完成");
        let action = orch.parse_response(&resp).unwrap();
        assert!(matches!(action, AgentAction::Final { .. }));
    }

    #[test]
    fn parse_react_malformed_treated_as_final() {
        // Lenient mode: plain rambling is treated as a final answer
        let orch = Orchestrator::with_protocol(Protocol::ReActText);
        let resp = CompletionResponse::react("just rambling with no labels");
        let action = orch.parse_response(&resp).unwrap();
        match action {
            AgentAction::Final { answer } => {
                assert!(answer.contains("rambling"));
            }
            _ => panic!("Expected Final, got {:?}", action),
        }
    }
}
