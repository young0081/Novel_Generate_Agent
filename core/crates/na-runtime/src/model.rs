//! The model-provider abstraction and a scriptable mock.
//!
//! The agent loop never talks to a concrete LLM SDK; it talks to a
//! [`ModelProvider`]. A request ([`CompletionRequest`]) carries the windowed
//! message context, the catalog of available tool specs, and which
//! [`Protocol`] the model should answer in (native structured tool calls, or
//! ReAct text). The response ([`CompletionResponse`]) carries the assistant text,
//! any parsed tool calls, and a [`FinishReason`].
//!
//! [`MockProvider`] is fully scriptable — built either from a fixed queue of
//! responses or from a closure `Fn(&CompletionRequest) -> CompletionResponse` —
//! so a complete agent loop can be driven deterministically offline in tests and
//! the demo, with no network.
//!
//! ## Object safety
//!
//! Async-fn-in-trait is not `dyn`-compatible, so [`ModelProvider::complete`]
//! returns a manual [`BoxFuture`] and the trait is `Send + Sync` — the runtime
//! holds providers as `&dyn ModelProvider` / `Arc<dyn ModelProvider>`.

use std::sync::Mutex;

use na_common::{CoreError, Result};
use na_tools::ToolSpec;
use serde::{Deserialize, Serialize};

use crate::message::{Message, ToolCallRequest};

/// A boxed, `Send` future with an explicit lifetime — the object-safe stand-in
/// for `async fn` in [`ModelProvider`].
pub type BoxFuture<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

/// How the model is asked to express tool use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Protocol {
    /// The provider returns structured tool calls directly (e.g. an API with a
    /// native `tool_calls` field).
    #[default]
    NativeToolCall,
    /// The provider returns a ReAct text block that the orchestrator parses.
    ReActText,
}

/// Why a completion stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    /// The model produced a final answer / stopped naturally.
    Stop,
    /// The model wants to call one or more tools.
    ToolUse,
    /// The model hit its output length limit.
    Length,
}

/// Sampling parameters for model generation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SamplingParams {
    /// Temperature (0.0 = deterministic, 2.0 = very random). Default: 1.0.
    #[serde(default = "default_temperature")]
    pub temperature: f32,
    /// Top-p (nucleus sampling, 0.0-1.0). Default: 1.0.
    #[serde(default = "default_top_p")]
    pub top_p: f32,
    /// Top-k (0 = disabled). Default: 0.
    #[serde(default)]
    pub top_k: u32,
    /// Presence penalty (-2.0 to 2.0). Default: 0.0.
    #[serde(default)]
    pub presence_penalty: f32,
    /// Frequency penalty (-2.0 to 2.0). Default: 0.0.
    #[serde(default)]
    pub frequency_penalty: f32,
}

fn default_temperature() -> f32 { 1.0 }
fn default_top_p() -> f32 { 1.0 }

impl Default for SamplingParams {
    fn default() -> Self {
        SamplingParams {
            temperature: 1.0,
            top_p: 1.0,
            top_k: 0,
            presence_penalty: 0.0,
            frequency_penalty: 0.0,
        }
    }
}

/// A request for one model completion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompletionRequest {
    /// The (already windowed) conversation context.
    pub messages: Vec<Message>,
    /// The tools the model may call this turn.
    pub tools: Vec<ToolSpec>,
    /// Which protocol the response should use.
    pub protocol: Protocol,
    /// Sampling parameters (temperature, top_p, etc.).
    #[serde(default)]
    pub sampling: SamplingParams,
}

impl CompletionRequest {
    /// Construct a request with default sampling.
    pub fn new(messages: Vec<Message>, tools: Vec<ToolSpec>, protocol: Protocol) -> Self {
        CompletionRequest {
            messages,
            tools,
            protocol,
            sampling: SamplingParams::default(),
        }
    }

    /// Construct a request with custom sampling parameters.
    pub fn with_sampling(
        messages: Vec<Message>,
        tools: Vec<ToolSpec>,
        protocol: Protocol,
        sampling: SamplingParams,
    ) -> Self {
        CompletionRequest {
            messages,
            tools,
            protocol,
            sampling,
        }
    }
}

/// A model completion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompletionResponse {
    /// The assistant's text. For ReAct this is the raw block; for native tool
    /// use it is any surrounding narration (often empty).
    pub text: String,
    /// Tool calls the model requested (empty for a plain answer).
    pub tool_calls: Vec<ToolCallRequest>,
    /// Why the completion stopped.
    pub finish: FinishReason,
}

impl CompletionResponse {
    /// A terminal text answer ([`FinishReason::Stop`], no tool calls).
    pub fn answer(text: impl Into<String>) -> Self {
        CompletionResponse {
            text: text.into(),
            tool_calls: Vec::new(),
            finish: FinishReason::Stop,
        }
    }

    /// A single-tool-call response ([`FinishReason::ToolUse`]).
    pub fn tool_call(call: ToolCallRequest) -> Self {
        CompletionResponse {
            text: String::new(),
            tool_calls: vec![call],
            finish: FinishReason::ToolUse,
        }
    }

    /// A multi-tool-call response ([`FinishReason::ToolUse`]).
    pub fn tool_calls(calls: Vec<ToolCallRequest>) -> Self {
        CompletionResponse {
            text: String::new(),
            tool_calls: calls,
            finish: FinishReason::ToolUse,
        }
    }

    /// A raw ReAct text response (the orchestrator will parse `text`). Reported
    /// as [`FinishReason::ToolUse`] so the loop inspects the block; the parse
    /// decides whether it is actually an action or a final answer.
    pub fn react(text: impl Into<String>) -> Self {
        CompletionResponse {
            text: text.into(),
            tool_calls: Vec::new(),
            finish: FinishReason::ToolUse,
        }
    }

    /// Attach surrounding narration text (builder style).
    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        self.text = text.into();
        self
    }

    /// Override the finish reason (builder style).
    pub fn with_finish(mut self, finish: FinishReason) -> Self {
        self.finish = finish;
        self
    }
}

/// A source of model completions. Object-safe (`dyn ModelProvider`) and
/// `Send + Sync`.
pub trait ModelProvider: Send + Sync {
    /// Produce a completion for `request`.
    fn complete<'a>(
        &'a self,
        request: CompletionRequest,
    ) -> BoxFuture<'a, Result<CompletionResponse>>;

    /// Produce a completion for `request`, streaming text deltas to `on_delta`
    /// as they are generated (token by token).
    ///
    /// The default implementation does NOT stream: it calls
    /// [`complete`](Self::complete) and emits the whole text once — so providers
    /// without native streaming (e.g. [`MockProvider`]) keep working unchanged.
    /// Real providers (see `HttpModelProvider`) override this with true SSE
    /// token streaming so a UI can render the answer as it is written.
    fn complete_streaming<'a>(
        &'a self,
        request: CompletionRequest,
        on_delta: &'a (dyn Fn(&str) + Send + Sync),
    ) -> BoxFuture<'a, Result<CompletionResponse>> {
        Box::pin(async move {
            let resp = self.complete(request).await?;
            if !resp.text.is_empty() {
                on_delta(&resp.text);
            }
            Ok(resp)
        })
    }

    /// A short provider name (for logging / audit).
    fn name(&self) -> &str;
}

/// The scripting backend of [`MockProvider`]: either a fixed queue popped one
/// response per call, or a closure that computes a response from the request.
enum Script {
    /// A queue of pre-baked responses (popped front-to-back).
    Queue(Mutex<std::collections::VecDeque<CompletionResponse>>),
    /// A function computing a response from the request.
    Func(Box<dyn Fn(&CompletionRequest) -> CompletionResponse + Send + Sync>),
}

/// A deterministic, offline [`ModelProvider`] for tests and the demo.
///
/// Construct it with [`from_responses`](MockProvider::from_responses) (a queue,
/// one popped per `complete` call) or [`from_fn`](MockProvider::from_fn) (a
/// closure). A queue that runs dry returns a [`CoreError::model`] so a runaway
/// loop surfaces clearly rather than hanging.
pub struct MockProvider {
    name: String,
    script: Script,
}

impl std::fmt::Debug for MockProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let kind = match &self.script {
            Script::Queue(q) => format!("queue(len={})", q.lock().map(|q| q.len()).unwrap_or(0)),
            Script::Func(_) => "func".to_string(),
        };
        f.debug_struct("MockProvider")
            .field("name", &self.name)
            .field("script", &kind)
            .finish()
    }
}

impl MockProvider {
    /// Build a provider from a fixed queue of responses. Each call to
    /// [`complete`](ModelProvider::complete) pops the next one; an empty queue
    /// errors.
    pub fn from_responses(responses: Vec<CompletionResponse>) -> Self {
        MockProvider {
            name: "mock".to_string(),
            script: Script::Queue(Mutex::new(responses.into_iter().collect())),
        }
    }

    /// Build a provider from a closure computing each response from the request.
    pub fn from_fn<F>(func: F) -> Self
    where
        F: Fn(&CompletionRequest) -> CompletionResponse + Send + Sync + 'static,
    {
        MockProvider {
            name: "mock_fn".to_string(),
            script: Script::Func(Box::new(func)),
        }
    }

    /// Set a custom provider name (builder style).
    pub fn named(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// Responses still queued (0 for a closure-backed provider).
    pub fn remaining(&self) -> usize {
        match &self.script {
            Script::Queue(q) => q.lock().map(|q| q.len()).unwrap_or(0),
            Script::Func(_) => 0,
        }
    }
}

impl ModelProvider for MockProvider {
    fn complete<'a>(
        &'a self,
        request: CompletionRequest,
    ) -> BoxFuture<'a, Result<CompletionResponse>> {
        Box::pin(async move {
            match &self.script {
                Script::Queue(q) => {
                    let mut q = q
                        .lock()
                        .map_err(|_| CoreError::internal("mock provider queue lock poisoned"))?;
                    q.pop_front().ok_or_else(|| {
                        CoreError::model("mock provider script exhausted (no more responses)")
                    })
                }
                Script::Func(f) => Ok(f(&request)),
            }
        })
    }

    fn name(&self) -> &str {
        &self.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use na_common::json;
    use na_common::ToolCallId;

    fn req() -> CompletionRequest {
        CompletionRequest::new(vec![Message::user("hi")], vec![], Protocol::NativeToolCall)
    }

    #[test]
    fn protocol_and_finish_serde() {
        assert_eq!(
            serde_json::to_string(&Protocol::ReActText).unwrap(),
            "\"re_act_text\""
        );
        assert_eq!(
            serde_json::to_string(&FinishReason::ToolUse).unwrap(),
            "\"tool_use\""
        );
    }

    #[test]
    fn response_constructors() {
        let a = CompletionResponse::answer("done");
        assert_eq!(a.finish, FinishReason::Stop);
        assert!(a.tool_calls.is_empty());

        let call = ToolCallRequest::with_id(
            ToolCallId::from_existing("c1"),
            "read_file",
            json!({ "path": "a" }),
        );
        let t = CompletionResponse::tool_call(call.clone());
        assert_eq!(t.finish, FinishReason::ToolUse);
        assert_eq!(t.tool_calls.len(), 1);
        assert_eq!(t.tool_calls[0].name, "read_file");

        let r = CompletionResponse::react("Action: x\nAction Input: {}");
        assert_eq!(r.finish, FinishReason::ToolUse);
        assert!(r.text.contains("Action"));
    }

    #[tokio::test]
    async fn mock_queue_pops_in_order_then_errors() {
        let p = MockProvider::from_responses(vec![
            CompletionResponse::answer("first"),
            CompletionResponse::answer("second"),
        ]);
        assert_eq!(p.remaining(), 2);
        assert_eq!(p.complete(req()).await.unwrap().text, "first");
        assert_eq!(p.complete(req()).await.unwrap().text, "second");
        assert_eq!(p.remaining(), 0);
        // Exhausted -> model error.
        let err = p.complete(req()).await.unwrap_err();
        assert!(err.is(na_common::ErrorKind::Model));
    }

    #[tokio::test]
    async fn mock_fn_computes_from_request() {
        let p = MockProvider::from_fn(|r: &CompletionRequest| {
            CompletionResponse::answer(format!("saw {} messages", r.messages.len()))
        });
        let out = p.complete(req()).await.unwrap();
        assert_eq!(out.text, "saw 1 messages");
        // A closure provider can be called any number of times.
        assert_eq!(p.complete(req()).await.unwrap().text, "saw 1 messages");
    }

    #[tokio::test]
    async fn provider_is_object_safe() {
        let p: Box<dyn ModelProvider> = Box::new(MockProvider::from_responses(vec![
            CompletionResponse::answer("x"),
        ]));
        assert_eq!(p.name(), "mock");
        assert_eq!(p.complete(req()).await.unwrap().text, "x");
    }

    #[test]
    fn request_round_trips_json() {
        let r = req();
        let s = serde_json::to_string(&r).unwrap();
        let back: CompletionRequest = serde_json::from_str(&s).unwrap();
        assert_eq!(r, back);
    }
}
