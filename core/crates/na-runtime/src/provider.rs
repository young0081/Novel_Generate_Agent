//! Real LLM providers + multi-provider/multi-model configuration.
//!
//! [`HttpModelProvider`] implements [`ModelProvider`](crate::model::ModelProvider)
//! against real APIs in two wire formats:
//! * **OpenAI-compatible** (`/chat/completions`) — covers OpenAI, DeepSeek, Kimi,
//!   Zhipu, OpenRouter, Ollama / LM Studio, and most providers.
//! * **Anthropic** (`/v1/messages`) — Claude.
//! * **Gemini** (`/v1beta/models/{model}:generateContent` /
//!   `:streamGenerateContent`) — Google Gemini.
//!
//! Configuration ([`ProviderConfig`] / [`ProviderSettings`]) is managed by
//! [`ProviderStore`], a small JSON-file store the GUI drives to add/edit/remove
//! providers and pick the active provider + model. Our internal [`Message`] /
//! [`ToolSpec`] shapes are mapped to each API's request format, and native
//! tool-calling responses are parsed back into [`CompletionResponse`].

use std::path::{Path, PathBuf};
use std::time::Duration;

use reqwest::StatusCode;
use serde::{Deserialize, Serialize};

use na_common::{json, CoreError, Json, Result, ToolCallId};
use na_tools::ToolSpec;

use crate::message::{Message, Role, ToolCallRequest};
use crate::model::{
    BoxFuture, CompletionRequest, CompletionResponse, FinishReason, ModelProvider, Protocol,
    SamplingParams,
};

/// The wire protocol a provider speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderProtocol {
    /// OpenAI-compatible `/chat/completions`.
    OpenAi,
    /// Anthropic `/v1/messages`.
    Anthropic,
    /// Google Gemini native `generateContent`.
    Gemini,
}

/// How an agent loop should ask this provider to use tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderToolMode {
    /// Prefer the app's default native tool-call loop.
    #[default]
    Auto,
    /// Force native provider tool/function calls.
    Native,
    /// Use a text-only ReAct prompt, for providers/models that reject tools.
    Text,
}

/// One configured provider (one endpoint + key + its models).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderConfig {
    /// Stable id (used to reference the provider).
    pub id: String,
    /// Human-readable name shown in the UI.
    pub name: String,
    /// Wire protocol.
    pub protocol: ProviderProtocol,
    /// Tool-call compatibility mode for agent tasks.
    #[serde(default)]
    pub tool_mode: ProviderToolMode,
    /// Base URL. OpenAI: e.g. `https://api.openai.com/v1`; Anthropic: `https://api.anthropic.com`.
    pub base_url: String,
    /// API key (stored locally).
    pub api_key: String,
    /// The models this provider exposes (user-managed list).
    pub models: Vec<String>,
    /// Optional preferred model.
    #[serde(default)]
    pub default_model: Option<String>,
    /// Optional max output tokens (Anthropic requires one; default 4096).
    #[serde(default)]
    pub max_tokens: Option<u32>,
    /// Default sampling parameters for this provider.
    #[serde(default)]
    pub sampling: SamplingParams,
}

impl ProviderConfig {
    /// Pick the agent-loop protocol for this provider.
    ///
    /// `Auto` stays conservative for OpenAI-compatible endpoints because many
    /// third-party APIs accept basic chat completions but reject tool schemas.
    /// Anthropic's official endpoint supports native tools, so it keeps them.
    pub fn agent_protocol(&self) -> Protocol {
        match self.tool_mode {
            ProviderToolMode::Native => Protocol::NativeToolCall,
            ProviderToolMode::Text => Protocol::ReActText,
            ProviderToolMode::Auto => match self.protocol {
                ProviderProtocol::OpenAi => Protocol::ReActText,
                ProviderProtocol::Anthropic | ProviderProtocol::Gemini => Protocol::NativeToolCall,
            },
        }
    }
}

/// The whole provider configuration: every provider + the active selection.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProviderSettings {
    /// All configured providers.
    #[serde(default)]
    pub providers: Vec<ProviderConfig>,
    /// Active provider id.
    #[serde(default)]
    pub active_provider: Option<String>,
    /// Active model name (within the active provider).
    #[serde(default)]
    pub active_model: Option<String>,
}

// ---------------------------------------------------------------------------
// Request building / response parsing (pure, unit-tested offline).
// ---------------------------------------------------------------------------

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let head: String = s.chars().take(max).collect();
        format!("{head}…")
    }
}

fn uses_native_tools(req: &CompletionRequest) -> bool {
    req.protocol == Protocol::NativeToolCall && !req.tools.is_empty()
}

fn provider_status_message(
    provider: &str,
    status: StatusCode,
    body: &str,
    req: &CompletionRequest,
) -> String {
    let mut msg = format!("供应商 {provider} 返回 {status}: {}", truncate(body, 400));
    if uses_native_tools(req) {
        msg.push_str(
            "。该模型可能不支持原生工具调用，请在“供应商”中把工具调用模式改为“自动兼容”或“文本工具”。",
        );
    }
    msg
}

fn should_retry_without_stream(status: StatusCode, body: &str) -> bool {
    let lower = body.to_lowercase();
    matches!(status.as_u16(), 400 | 404 | 405 | 422 | 500 | 501)
        || lower.contains("stream")
        || lower.contains("sse")
        || lower.contains("bad_response_status_code")
}

fn gemini_model_path(model: &str) -> String {
    model
        .split('/')
        .map(|part| part.replace(':', "%3A"))
        .collect::<Vec<_>>()
        .join("/")
}

fn react_tool_call_text(thought: &str, call: &ToolCallRequest) -> String {
    let mut text = String::new();
    if !thought.trim().is_empty() {
        text.push_str("Thought: ");
        text.push_str(thought.trim());
        text.push('\n');
    }
    text.push_str("Action: ");
    text.push_str(&call.name);
    text.push_str("\nAction Input: ");
    text.push_str(&call.args.to_string());
    text
}

fn react_observation_text(content: &str) -> String {
    format!("Observation: {content}")
}

/// Map our messages to OpenAI chat messages.
fn openai_messages(msgs: &[Message], protocol: Protocol) -> Vec<Json> {
    let mut out = Vec::with_capacity(msgs.len());
    for m in msgs {
        match m.role {
            Role::System => out.push(json!({ "role": "system", "content": m.content })),
            Role::User => out.push(json!({ "role": "user", "content": m.content })),
            Role::Assistant => {
                if let Some(tc) = &m.tool_call {
                    if protocol == Protocol::ReActText {
                        out.push(json!({
                            "role": "assistant",
                            "content": react_tool_call_text(&m.content, tc)
                        }));
                    } else {
                        out.push(json!({
                            "role": "assistant",
                            "content": if m.content.is_empty() { Json::Null } else { json!(m.content) },
                            "tool_calls": [{
                                "id": tc.id.as_str(),
                                "type": "function",
                                "function": { "name": tc.name, "arguments": tc.args.to_string() }
                            }]
                        }));
                    }
                } else {
                    out.push(json!({ "role": "assistant", "content": m.content }));
                }
            }
            Role::Tool => {
                if protocol == Protocol::ReActText {
                    out.push(json!({
                        "role": "user",
                        "content": react_observation_text(&m.content)
                    }));
                } else {
                    let id = m
                        .tool_result
                        .as_ref()
                        .map(|r| r.call_id.as_str().to_string())
                        .unwrap_or_default();
                    out.push(json!({ "role": "tool", "tool_call_id": id, "content": m.content }));
                }
            }
        }
    }
    out
}

/// Build an OpenAI `/chat/completions` request body.
pub fn build_openai_body(model: &str, max_tokens: u32, req: &CompletionRequest) -> Json {
    let mut body = json!({
        "model": model,
        "messages": openai_messages(&req.messages, req.protocol),
        "max_tokens": max_tokens,
        "temperature": req.sampling.temperature,
        "top_p": req.sampling.top_p,
    });

    // Only add top_k if non-zero (not all providers support it)
    if req.sampling.top_k > 0 {
        body["top_k"] = json!(req.sampling.top_k);
    }

    // Add penalties if non-zero
    if req.sampling.presence_penalty != 0.0 {
        body["presence_penalty"] = json!(req.sampling.presence_penalty);
    }
    if req.sampling.frequency_penalty != 0.0 {
        body["frequency_penalty"] = json!(req.sampling.frequency_penalty);
    }

    if req.protocol == Protocol::NativeToolCall && !req.tools.is_empty() {
        let tools: Vec<Json> = req
            .tools
            .iter()
            .map(|t: &ToolSpec| {
                json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.input_schema
                    }
                })
            })
            .collect();
        body["tools"] = json!(tools);
        body["tool_choice"] = json!("auto");
    }
    body
}

/// Parse an OpenAI chat completion response.
pub fn parse_openai_response(v: &Json) -> Result<CompletionResponse> {
    if let Some(err) = v.get("error") {
        let msg = err
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("unknown provider error");
        return Err(CoreError::model(format!("provider error: {msg}")));
    }
    let choice = v
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|a| a.first())
        .ok_or_else(|| CoreError::model("响应中没有 choices"))?;
    let msg = choice
        .get("message")
        .ok_or_else(|| CoreError::model("响应中没有 message"))?;
    let text = msg
        .get("content")
        .and_then(|c| c.as_str())
        .unwrap_or("")
        .to_string();

    let mut tool_calls = Vec::new();
    if let Some(tcs) = msg.get("tool_calls").and_then(|t| t.as_array()) {
        for tc in tcs {
            let id = tc
                .get("id")
                .and_then(|i| i.as_str())
                .unwrap_or("")
                .to_string();
            let func = tc.get("function");
            let name = func
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())
                .unwrap_or("")
                .to_string();
            let args_raw = func
                .and_then(|f| f.get("arguments"))
                .and_then(|a| a.as_str())
                .unwrap_or("{}");
            let args = serde_json::from_str::<Json>(args_raw)
                .unwrap_or_else(|_| json!({ "_raw": args_raw }));
            if !name.is_empty() {
                let cid = if id.is_empty() {
                    ToolCallId::new()
                } else {
                    ToolCallId::from_existing(id)
                };
                tool_calls.push(ToolCallRequest::with_id(cid, name, args));
            }
        }
    }

    let finish = match choice.get("finish_reason").and_then(|f| f.as_str()) {
        Some("tool_calls") => FinishReason::ToolUse,
        Some("length") => FinishReason::Length,
        _ if !tool_calls.is_empty() => FinishReason::ToolUse,
        _ => FinishReason::Stop,
    };

    Ok(CompletionResponse {
        text,
        tool_calls,
        finish,
    })
}

/// Append a content block to the running message sequence, merging consecutive
/// same-role turns (Anthropic requires strictly alternating roles).
fn anthropic_push(seq: &mut Vec<(&'static str, Vec<Json>)>, role: &'static str, block: Json) {
    if let Some(last) = seq.last_mut() {
        if last.0 == role {
            last.1.push(block);
            return;
        }
    }
    seq.push((role, vec![block]));
}

/// Map our messages to Anthropic (system string + message blocks).
fn anthropic_messages(msgs: &[Message], protocol: Protocol) -> (String, Vec<Json>) {
    let mut system = String::new();
    let mut seq: Vec<(&'static str, Vec<Json>)> = Vec::new();

    for m in msgs {
        match m.role {
            Role::System => {
                if !system.is_empty() {
                    system.push_str("\n\n");
                }
                system.push_str(&m.content);
            }
            Role::User => {
                anthropic_push(
                    &mut seq,
                    "user",
                    json!({ "type": "text", "text": m.content }),
                );
            }
            Role::Assistant => {
                if let Some(tc) = &m.tool_call {
                    if protocol == Protocol::ReActText {
                        anthropic_push(
                            &mut seq,
                            "assistant",
                            json!({ "type": "text", "text": react_tool_call_text(&m.content, tc) }),
                        );
                    } else {
                        if !m.content.is_empty() {
                            anthropic_push(
                                &mut seq,
                                "assistant",
                                json!({ "type": "text", "text": m.content }),
                            );
                        }
                        anthropic_push(
                            &mut seq,
                            "assistant",
                            json!({ "type": "tool_use", "id": tc.id.as_str(), "name": tc.name, "input": tc.args }),
                        );
                    }
                } else if !m.content.is_empty() {
                    anthropic_push(
                        &mut seq,
                        "assistant",
                        json!({ "type": "text", "text": m.content }),
                    );
                }
            }
            Role::Tool => {
                if protocol == Protocol::ReActText {
                    anthropic_push(
                        &mut seq,
                        "user",
                        json!({ "type": "text", "text": react_observation_text(&m.content) }),
                    );
                } else {
                    let id = m
                        .tool_result
                        .as_ref()
                        .map(|r| r.call_id.as_str().to_string())
                        .unwrap_or_default();
                    anthropic_push(
                        &mut seq,
                        "user",
                        json!({ "type": "tool_result", "tool_use_id": id, "content": m.content }),
                    );
                }
            }
        }
    }

    let messages = seq
        .into_iter()
        .map(|(role, blocks)| json!({ "role": role, "content": blocks }))
        .collect();
    (system, messages)
}

/// Build an Anthropic `/v1/messages` request body.
pub fn build_anthropic_body(model: &str, max_tokens: u32, req: &CompletionRequest) -> Json {
    let (system, messages) = anthropic_messages(&req.messages, req.protocol);
    let mut body = json!({
        "model": model,
        "max_tokens": max_tokens,
        "messages": messages,
        "temperature": req.sampling.temperature,
        "top_p": req.sampling.top_p,
    });

    // Anthropic supports top_k
    if req.sampling.top_k > 0 {
        body["top_k"] = json!(req.sampling.top_k);
    }

    if !system.is_empty() {
        body["system"] = json!(system);
    }
    if req.protocol == Protocol::NativeToolCall && !req.tools.is_empty() {
        let tools: Vec<Json> = req
            .tools
            .iter()
            .map(|t: &ToolSpec| {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.input_schema
                })
            })
            .collect();
        body["tools"] = json!(tools);
    }
    body
}

/// Parse an Anthropic messages response.
pub fn parse_anthropic_response(v: &Json) -> Result<CompletionResponse> {
    if v.get("type").and_then(|t| t.as_str()) == Some("error") {
        let msg = v
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
            .unwrap_or("unknown provider error");
        return Err(CoreError::model(format!("provider error: {msg}")));
    }
    let content = v
        .get("content")
        .and_then(|c| c.as_array())
        .ok_or_else(|| CoreError::model("响应中没有 content"))?;

    let mut text = String::new();
    let mut tool_calls = Vec::new();
    for block in content {
        match block.get("type").and_then(|t| t.as_str()) {
            Some("text") => {
                if let Some(t) = block.get("text").and_then(|t| t.as_str()) {
                    text.push_str(t);
                }
            }
            Some("tool_use") => {
                let id = block
                    .get("id")
                    .and_then(|i| i.as_str())
                    .unwrap_or("")
                    .to_string();
                let name = block
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_string();
                let input = block.get("input").cloned().unwrap_or_else(|| json!({}));
                if !name.is_empty() {
                    let cid = if id.is_empty() {
                        ToolCallId::new()
                    } else {
                        ToolCallId::from_existing(id)
                    };
                    tool_calls.push(ToolCallRequest::with_id(cid, name, input));
                }
            }
            _ => {}
        }
    }

    let finish = match v.get("stop_reason").and_then(|s| s.as_str()) {
        Some("tool_use") => FinishReason::ToolUse,
        Some("max_tokens") => FinishReason::Length,
        _ if !tool_calls.is_empty() => FinishReason::ToolUse,
        _ => FinishReason::Stop,
    };

    Ok(CompletionResponse {
        text,
        tool_calls,
        finish,
    })
}

fn gemini_function_declaration(spec: &ToolSpec) -> Json {
    json!({
        "name": spec.name,
        "description": spec.description,
        "parameters": spec.input_schema,
    })
}

fn gemini_text_part(text: &str) -> Json {
    json!({ "text": text })
}

fn gemini_function_call_part(call: &ToolCallRequest) -> Json {
    json!({
        "functionCall": {
            "name": call.name,
            "args": call.args,
        }
    })
}

fn gemini_function_response_part(msg: &Message) -> Json {
    let name = msg
        .tool_result
        .as_ref()
        .map(|r| r.name.clone())
        .unwrap_or_else(|| "tool".to_string());
    json!({
        "functionResponse": {
            "name": name,
            "response": {
                "content": msg.content,
                "ok": msg.tool_result.as_ref().map(|r| r.ok).unwrap_or(true),
            }
        }
    })
}

fn gemini_contents(msgs: &[Message], protocol: Protocol) -> (Option<String>, Vec<Json>) {
    let mut system_parts: Vec<String> = Vec::new();
    let mut contents: Vec<Json> = Vec::new();

    for m in msgs {
        match m.role {
            Role::System => system_parts.push(m.content.clone()),
            Role::User => contents.push(json!({
                "role": "user",
                "parts": [gemini_text_part(&m.content)]
            })),
            Role::Assistant => {
                let mut parts = Vec::new();
                if let Some(call) = &m.tool_call {
                    if protocol == Protocol::ReActText {
                        parts.push(gemini_text_part(&react_tool_call_text(&m.content, call)));
                    } else {
                        if !m.content.trim().is_empty() {
                            parts.push(gemini_text_part(&m.content));
                        }
                        parts.push(gemini_function_call_part(call));
                    }
                } else if !m.content.is_empty() {
                    parts.push(gemini_text_part(&m.content));
                }
                if !parts.is_empty() {
                    contents.push(json!({ "role": "model", "parts": parts }));
                }
            }
            Role::Tool => {
                let part = if protocol == Protocol::ReActText {
                    gemini_text_part(&react_observation_text(&m.content))
                } else {
                    gemini_function_response_part(m)
                };
                contents.push(json!({ "role": "user", "parts": [part] }));
            }
        }
    }

    let system = if system_parts.is_empty() {
        None
    } else {
        Some(system_parts.join("\n\n"))
    };
    (system, contents)
}

/// Build a Gemini native `generateContent` request body.
pub fn build_gemini_body(max_tokens: u32, req: &CompletionRequest) -> Json {
    let (system, contents) = gemini_contents(&req.messages, req.protocol);
    let mut generation_config = json!({
        "maxOutputTokens": max_tokens,
        "temperature": req.sampling.temperature,
        "topP": req.sampling.top_p,
    });

    // Gemini supports topK
    if req.sampling.top_k > 0 {
        generation_config["topK"] = json!(req.sampling.top_k);
    }

    let mut body = json!({
        "contents": contents,
        "generationConfig": generation_config,
    });

    if let Some(system) = system {
        body["systemInstruction"] = json!({
            "parts": [{ "text": system }]
        });
    }
    if req.protocol == Protocol::NativeToolCall && !req.tools.is_empty() {
        let declarations: Vec<Json> = req.tools.iter().map(gemini_function_declaration).collect();
        body["tools"] = json!([{ "functionDeclarations": declarations }]);
        body["toolConfig"] = json!({
            "functionCallingConfig": { "mode": "AUTO" }
        });
    }
    body
}

/// Parse a Gemini native `generateContent` response.
pub fn parse_gemini_response(v: &Json) -> Result<CompletionResponse> {
    if let Some(err) = v.get("error") {
        let msg = err
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("unknown provider error");
        return Err(CoreError::model(format!("provider error: {msg}")));
    }

    let candidate = v
        .get("candidates")
        .and_then(|c| c.as_array())
        .and_then(|a| a.first())
        .ok_or_else(|| CoreError::model("响应中没有 candidates"))?;
    let parts = candidate
        .get("content")
        .and_then(|c| c.get("parts"))
        .and_then(|p| p.as_array())
        .ok_or_else(|| CoreError::model("响应中没有 content.parts"))?;

    let mut text = String::new();
    let mut tool_calls = Vec::new();
    for part in parts {
        if let Some(t) = part.get("text").and_then(|t| t.as_str()) {
            text.push_str(t);
        }
        if let Some(fc) = part.get("functionCall") {
            let name = fc
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or("")
                .to_string();
            let args = fc.get("args").cloned().unwrap_or_else(|| json!({}));
            if !name.is_empty() {
                tool_calls.push(ToolCallRequest::new(name, args));
            }
        }
    }

    let finish = if !tool_calls.is_empty() {
        FinishReason::ToolUse
    } else {
        match candidate.get("finishReason").and_then(|f| f.as_str()) {
            Some("MAX_TOKENS") => FinishReason::Length,
            _ => FinishReason::Stop,
        }
    };

    Ok(CompletionResponse {
        text,
        tool_calls,
        finish,
    })
}

// ---------------------------------------------------------------------------
// Streaming (SSE) parsing — pure, unit-tested offline.
// ---------------------------------------------------------------------------

/// A tool call accumulated across streaming deltas (id / name / argument JSON
/// fragments arrive piecemeal and must be concatenated).
#[derive(Default)]
struct ToolAccum {
    id: String,
    name: String,
    args: String,
}

/// Drain whole `\n`-terminated lines from a byte buffer, leaving any trailing
/// partial line in place. Decoding per complete line keeps multi-byte UTF-8
/// (e.g. Chinese) intact across network chunk boundaries.
fn drain_lines(buf: &mut Vec<u8>) -> Vec<String> {
    let mut lines = Vec::new();
    while let Some(pos) = buf.iter().position(|&b| b == b'\n') {
        let line: Vec<u8> = buf.drain(..=pos).collect();
        let s = String::from_utf8_lossy(&line);
        lines.push(s.trim_end_matches(['\n', '\r']).to_string());
    }
    lines
}

/// Extract the payload of an SSE `data:` line (trimmed), if this is one.
fn sse_data(line: &str) -> Option<&str> {
    line.trim_start().strip_prefix("data:").map(|d| d.trim())
}

/// Fold one OpenAI streaming chunk into the running accumulators.
fn openai_stream_event(
    d: &Json,
    text: &mut String,
    tools: &mut Vec<ToolAccum>,
    finish: &mut Option<FinishReason>,
    on_delta: &dyn Fn(&str),
) {
    let Some(choice) = d
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|a| a.first())
    else {
        return;
    };
    if let Some(delta) = choice.get("delta") {
        if let Some(content) = delta.get("content").and_then(|c| c.as_str()) {
            if !content.is_empty() {
                text.push_str(content);
                on_delta(content);
            }
        }
        if let Some(tcs) = delta.get("tool_calls").and_then(|t| t.as_array()) {
            for tc in tcs {
                let idx = tc.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
                while tools.len() <= idx {
                    tools.push(ToolAccum::default());
                }
                let slot = &mut tools[idx];
                if let Some(id) = tc.get("id").and_then(|i| i.as_str()) {
                    if !id.is_empty() {
                        slot.id = id.to_string();
                    }
                }
                if let Some(f) = tc.get("function") {
                    if let Some(name) = f.get("name").and_then(|n| n.as_str()) {
                        slot.name.push_str(name);
                    }
                    if let Some(args) = f.get("arguments").and_then(|a| a.as_str()) {
                        slot.args.push_str(args);
                    }
                }
            }
        }
    }
    if let Some(fr) = choice.get("finish_reason").and_then(|f| f.as_str()) {
        *finish = Some(match fr {
            "tool_calls" => FinishReason::ToolUse,
            "length" => FinishReason::Length,
            _ => FinishReason::Stop,
        });
    }
}

/// Fold one Anthropic streaming event into the running accumulators.
fn anthropic_stream_event(
    d: &Json,
    text: &mut String,
    tools: &mut Vec<ToolAccum>,
    finish: &mut Option<FinishReason>,
    on_delta: &dyn Fn(&str),
) {
    match d.get("type").and_then(|t| t.as_str()) {
        Some("content_block_start") => {
            let idx = d.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
            if let Some(cb) = d.get("content_block") {
                if cb.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                    while tools.len() <= idx {
                        tools.push(ToolAccum::default());
                    }
                    let slot = &mut tools[idx];
                    if let Some(id) = cb.get("id").and_then(|i| i.as_str()) {
                        slot.id = id.to_string();
                    }
                    if let Some(name) = cb.get("name").and_then(|n| n.as_str()) {
                        slot.name = name.to_string();
                    }
                }
            }
        }
        Some("content_block_delta") => {
            let idx = d.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
            if let Some(delta) = d.get("delta") {
                match delta.get("type").and_then(|t| t.as_str()) {
                    Some("text_delta") => {
                        if let Some(t) = delta.get("text").and_then(|t| t.as_str()) {
                            text.push_str(t);
                            on_delta(t);
                        }
                    }
                    Some("input_json_delta") => {
                        if let Some(pj) = delta.get("partial_json").and_then(|p| p.as_str()) {
                            while tools.len() <= idx {
                                tools.push(ToolAccum::default());
                            }
                            tools[idx].args.push_str(pj);
                        }
                    }
                    _ => {}
                }
            }
        }
        Some("message_delta") => {
            if let Some(sr) = d
                .get("delta")
                .and_then(|x| x.get("stop_reason"))
                .and_then(|s| s.as_str())
            {
                *finish = Some(match sr {
                    "tool_use" => FinishReason::ToolUse,
                    "max_tokens" => FinishReason::Length,
                    _ => FinishReason::Stop,
                });
            }
        }
        _ => {}
    }
}

/// Fold one Gemini `streamGenerateContent` SSE chunk into the running
/// accumulators. Each chunk has the same top-level shape as a partial
/// `GenerateContentResponse`.
fn gemini_stream_event(
    d: &Json,
    text: &mut String,
    tools: &mut Vec<ToolAccum>,
    finish: &mut Option<FinishReason>,
    on_delta: &dyn Fn(&str),
) -> Result<()> {
    if let Some(err) = d.get("error") {
        let msg = err
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("unknown provider error");
        return Err(CoreError::model(format!("provider error: {msg}")));
    }

    let Some(candidate) = d
        .get("candidates")
        .and_then(|c| c.as_array())
        .and_then(|a| a.first())
    else {
        return Ok(());
    };

    if let Some(parts) = candidate
        .get("content")
        .and_then(|c| c.get("parts"))
        .and_then(|p| p.as_array())
    {
        for part in parts {
            if let Some(t) = part.get("text").and_then(|t| t.as_str()) {
                if !t.is_empty() {
                    text.push_str(t);
                    on_delta(t);
                }
            }
            if let Some(fc) = part.get("functionCall") {
                let name = fc
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_string();
                if !name.is_empty() {
                    let args = fc.get("args").cloned().unwrap_or_else(|| json!({}));
                    tools.push(ToolAccum {
                        id: String::new(),
                        name,
                        args: args.to_string(),
                    });
                }
            }
        }
    }

    if let Some(fr) = candidate.get("finishReason").and_then(|f| f.as_str()) {
        *finish = Some(match fr {
            "MAX_TOKENS" => FinishReason::Length,
            _ => FinishReason::Stop,
        });
    }

    Ok(())
}

/// Build the final [`CompletionResponse`] from streamed accumulators.
fn finish_accum(
    text: String,
    tools: Vec<ToolAccum>,
    finish: Option<FinishReason>,
) -> CompletionResponse {
    let mut tool_calls = Vec::new();
    for t in tools {
        if t.name.is_empty() {
            continue;
        }
        let args = if t.args.trim().is_empty() {
            json!({})
        } else {
            serde_json::from_str::<Json>(&t.args).unwrap_or_else(|_| json!({ "_raw": t.args }))
        };
        let cid = if t.id.is_empty() {
            ToolCallId::new()
        } else {
            ToolCallId::from_existing(t.id)
        };
        tool_calls.push(ToolCallRequest::with_id(cid, t.name, args));
    }
    let finish = finish.unwrap_or(if tool_calls.is_empty() {
        FinishReason::Stop
    } else {
        FinishReason::ToolUse
    });
    // A finish of Stop alongside pending tool calls still means "use tools".
    let finish = if !tool_calls.is_empty() && finish == FinishReason::Stop {
        FinishReason::ToolUse
    } else {
        finish
    };
    CompletionResponse {
        text,
        tool_calls,
        finish,
    }
}

// ---------------------------------------------------------------------------
// The live HTTP provider.
// ---------------------------------------------------------------------------

/// A real model provider that calls a configured HTTP endpoint.
pub struct HttpModelProvider {
    client: reqwest::Client,
    config: ProviderConfig,
    model: String,
    max_tokens: u32,
}

impl std::fmt::Debug for HttpModelProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpModelProvider")
            .field("provider", &self.config.name)
            .field("protocol", &self.config.protocol)
            .field("model", &self.model)
            .finish_non_exhaustive()
    }
}

impl HttpModelProvider {
    /// Build a provider for `config` using `model`.
    pub fn new(config: ProviderConfig, model: impl Into<String>) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(180))
            .user_agent("novel-generate-team/0.1")
            .build()
            .map_err(|e| CoreError::model(format!("无法创建 HTTP 客户端: {e}")))?;
        let max_tokens = config.max_tokens.unwrap_or(4096);
        Ok(HttpModelProvider {
            client,
            config,
            model: model.into(),
            max_tokens,
        })
    }

    async fn call_openai(&self, req: CompletionRequest) -> Result<CompletionResponse> {
        let url = format!(
            "{}/chat/completions",
            self.config.base_url.trim_end_matches('/')
        );
        let body = build_openai_body(&self.model, self.max_tokens, &req);
        let resp = self
            .client
            .post(&url)
            .bearer_auth(&self.config.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| CoreError::model(format!("请求失败: {e}")))?;
        let status = resp.status();
        let txt = resp
            .text()
            .await
            .map_err(|e| CoreError::model(format!("读取响应失败: {e}")))?;
        if !status.is_success() {
            return Err(CoreError::model(provider_status_message(
                &self.config.name,
                status,
                &txt,
                &req,
            )));
        }
        let v: Json = serde_json::from_str(&txt)
            .map_err(|e| CoreError::model(format!("响应不是合法 JSON: {e}")))?;
        parse_openai_response(&v)
    }

    async fn call_anthropic(&self, req: CompletionRequest) -> Result<CompletionResponse> {
        let url = format!("{}/v1/messages", self.config.base_url.trim_end_matches('/'));
        let body = build_anthropic_body(&self.model, self.max_tokens, &req);
        let resp = self
            .client
            .post(&url)
            .header("x-api-key", &self.config.api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&body)
            .send()
            .await
            .map_err(|e| CoreError::model(format!("请求失败: {e}")))?;
        let status = resp.status();
        let txt = resp
            .text()
            .await
            .map_err(|e| CoreError::model(format!("读取响应失败: {e}")))?;
        if !status.is_success() {
            return Err(CoreError::model(provider_status_message(
                &self.config.name,
                status,
                &txt,
                &req,
            )));
        }
        let v: Json = serde_json::from_str(&txt)
            .map_err(|e| CoreError::model(format!("响应不是合法 JSON: {e}")))?;
        parse_anthropic_response(&v)
    }

    async fn call_gemini(&self, req: CompletionRequest) -> Result<CompletionResponse> {
        let url = format!(
            "{}/v1beta/models/{}:generateContent",
            self.config.base_url.trim_end_matches('/'),
            gemini_model_path(&self.model)
        );
        let body = build_gemini_body(self.max_tokens, &req);
        let resp = self
            .client
            .post(&url)
            .header("x-goog-api-key", &self.config.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| CoreError::model(format!("请求失败: {e}")))?;
        let status = resp.status();
        let txt = resp
            .text()
            .await
            .map_err(|e| CoreError::model(format!("读取响应失败: {e}")))?;
        if !status.is_success() {
            return Err(CoreError::model(provider_status_message(
                &self.config.name,
                status,
                &txt,
                &req,
            )));
        }
        let v: Json = serde_json::from_str(&txt)
            .map_err(|e| CoreError::model(format!("响应不是合法 JSON: {e}")))?;
        parse_gemini_response(&v)
    }

    /// OpenAI-compatible `/chat/completions` with `stream: true` (SSE), emitting
    /// each content fragment to `on_delta` as it arrives.
    async fn call_openai_stream(
        &self,
        req: CompletionRequest,
        on_delta: &(dyn Fn(&str) + Send + Sync),
    ) -> Result<CompletionResponse> {
        let url = format!(
            "{}/chat/completions",
            self.config.base_url.trim_end_matches('/')
        );
        let mut body = build_openai_body(&self.model, self.max_tokens, &req);
        body["stream"] = json!(true);
        let mut resp = self
            .client
            .post(&url)
            .bearer_auth(&self.config.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| CoreError::model(format!("请求失败: {e}")))?;
        let status = resp.status();
        if !status.is_success() {
            let txt = resp.text().await.unwrap_or_default();
            if should_retry_without_stream(status, &txt) {
                return self.call_openai(req).await;
            }
            return Err(CoreError::model(provider_status_message(
                &self.config.name,
                status,
                &txt,
                &req,
            )));
        }

        let mut text = String::new();
        let mut tools: Vec<ToolAccum> = Vec::new();
        let mut finish: Option<FinishReason> = None;
        let mut buf: Vec<u8> = Vec::new();
        let mut done = false;
        let mut saw_event = false;
        while let Some(chunk) = resp
            .chunk()
            .await
            .map_err(|e| CoreError::model(format!("读取流失败: {e}")))?
        {
            buf.extend_from_slice(&chunk);
            for line in drain_lines(&mut buf) {
                if let Some(data) = sse_data(&line) {
                    if data == "[DONE]" {
                        saw_event = true;
                        done = true;
                        break;
                    }
                    if let Ok(d) = serde_json::from_str::<Json>(data) {
                        saw_event = true;
                        openai_stream_event(&d, &mut text, &mut tools, &mut finish, on_delta);
                    }
                }
            }
            if done {
                break;
            }
        }
        // Some endpoints ignore `stream: true` and return a normal JSON body.
        // If we never parsed an SSE event, fall back to a plain completion.
        if !saw_event {
            return self.call_openai(req).await;
        }
        Ok(finish_accum(text, tools, finish))
    }

    /// Anthropic `/v1/messages` with `stream: true` (SSE), emitting each text
    /// fragment to `on_delta` as it arrives.
    async fn call_anthropic_stream(
        &self,
        req: CompletionRequest,
        on_delta: &(dyn Fn(&str) + Send + Sync),
    ) -> Result<CompletionResponse> {
        let url = format!("{}/v1/messages", self.config.base_url.trim_end_matches('/'));
        let mut body = build_anthropic_body(&self.model, self.max_tokens, &req);
        body["stream"] = json!(true);
        let mut resp = self
            .client
            .post(&url)
            .header("x-api-key", &self.config.api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&body)
            .send()
            .await
            .map_err(|e| CoreError::model(format!("请求失败: {e}")))?;
        let status = resp.status();
        if !status.is_success() {
            let txt = resp.text().await.unwrap_or_default();
            if should_retry_without_stream(status, &txt) {
                return self.call_anthropic(req).await;
            }
            return Err(CoreError::model(provider_status_message(
                &self.config.name,
                status,
                &txt,
                &req,
            )));
        }

        let mut text = String::new();
        let mut tools: Vec<ToolAccum> = Vec::new();
        let mut finish: Option<FinishReason> = None;
        let mut buf: Vec<u8> = Vec::new();
        let mut saw_event = false;
        while let Some(chunk) = resp
            .chunk()
            .await
            .map_err(|e| CoreError::model(format!("读取流失败: {e}")))?
        {
            buf.extend_from_slice(&chunk);
            for line in drain_lines(&mut buf) {
                if let Some(data) = sse_data(&line) {
                    if let Ok(d) = serde_json::from_str::<Json>(data) {
                        saw_event = true;
                        anthropic_stream_event(&d, &mut text, &mut tools, &mut finish, on_delta);
                    }
                }
            }
        }
        // Some endpoints ignore `stream: true` and return a normal JSON body.
        // If we never parsed an SSE event, fall back to a plain completion.
        if !saw_event {
            return self.call_anthropic(req).await;
        }
        Ok(finish_accum(text, tools, finish))
    }

    async fn call_gemini_stream(
        &self,
        req: CompletionRequest,
        on_delta: &(dyn Fn(&str) + Send + Sync),
    ) -> Result<CompletionResponse> {
        let url = format!(
            "{}/v1beta/models/{}:streamGenerateContent?alt=sse",
            self.config.base_url.trim_end_matches('/'),
            gemini_model_path(&self.model)
        );
        let body = build_gemini_body(self.max_tokens, &req);
        let mut resp = self
            .client
            .post(&url)
            .header("x-goog-api-key", &self.config.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| CoreError::model(format!("请求失败: {e}")))?;
        let status = resp.status();
        if !status.is_success() {
            let txt = resp.text().await.unwrap_or_default();
            if should_retry_without_stream(status, &txt) {
                return self.call_gemini(req).await;
            }
            return Err(CoreError::model(provider_status_message(
                &self.config.name,
                status,
                &txt,
                &req,
            )));
        }

        let mut text = String::new();
        let mut tools: Vec<ToolAccum> = Vec::new();
        let mut finish: Option<FinishReason> = None;
        let mut buf: Vec<u8> = Vec::new();
        let mut done = false;
        let mut saw_event = false;
        while let Some(chunk) = resp
            .chunk()
            .await
            .map_err(|e| CoreError::model(format!("读取流失败: {e}")))?
        {
            buf.extend_from_slice(&chunk);
            for line in drain_lines(&mut buf) {
                if let Some(data) = sse_data(&line) {
                    if data == "[DONE]" {
                        saw_event = true;
                        done = true;
                        break;
                    }
                    if let Ok(d) = serde_json::from_str::<Json>(data) {
                        saw_event = true;
                        gemini_stream_event(&d, &mut text, &mut tools, &mut finish, on_delta)?;
                    }
                }
            }
            if done {
                break;
            }
        }
        // If an endpoint ignores SSE and returns a normal/non-SSE body, retry
        // through the plain Gemini completion path rather than returning empty.
        if !saw_event {
            return self.call_gemini(req).await;
        }
        Ok(finish_accum(text, tools, finish))
    }
}

/// Returns true for transient network errors worth retrying (connection refused,
/// timeout, DNS failure). API-level errors (4xx/5xx) are not transient.
fn is_transient(e: &na_common::CoreError) -> bool {
    let msg = e.to_string().to_lowercase();
    msg.contains("error sending request")
        || msg.contains("connection refused")
        || msg.contains("connection reset")
        || msg.contains("timed out")
        || msg.contains("dns error")
        || msg.contains("failed to connect")
}

impl ModelProvider for HttpModelProvider {
    fn complete<'a>(
        &'a self,
        request: CompletionRequest,
    ) -> BoxFuture<'a, Result<CompletionResponse>> {
        Box::pin(async move {
            // Retry up to 2 times on transient network errors (connection refused,
            // timeout, DNS failure). 4xx/5xx API errors are not retried.
            let mut last_err = None;
            for attempt in 0u8..3 {
                if attempt > 0 {
                    tokio::time::sleep(std::time::Duration::from_millis(
                        500 * (attempt as u64),
                    ))
                    .await;
                }
                let result = match self.config.protocol {
                    ProviderProtocol::OpenAi => self.call_openai(request.clone()).await,
                    ProviderProtocol::Anthropic => self.call_anthropic(request.clone()).await,
                    ProviderProtocol::Gemini => self.call_gemini(request.clone()).await,
                };
                match result {
                    Ok(resp) => return Ok(resp),
                    Err(e) if is_transient(&e) && attempt < 2 => {
                        last_err = Some(e);
                    }
                    Err(e) => return Err(e),
                }
            }
            Err(last_err.unwrap())
        })
    }

    fn complete_streaming<'a>(
        &'a self,
        request: CompletionRequest,
        on_delta: &'a (dyn Fn(&str) + Send + Sync),
    ) -> BoxFuture<'a, Result<CompletionResponse>> {
        Box::pin(async move {
            let mut last_err = None;
            for attempt in 0u8..3 {
                if attempt > 0 {
                    tokio::time::sleep(std::time::Duration::from_millis(
                        500 * (attempt as u64),
                    ))
                    .await;
                }
                let result = match self.config.protocol {
                    ProviderProtocol::OpenAi => {
                        self.call_openai_stream(request.clone(), on_delta).await
                    }
                    ProviderProtocol::Anthropic => {
                        self.call_anthropic_stream(request.clone(), on_delta).await
                    }
                    ProviderProtocol::Gemini => {
                        self.call_gemini_stream(request.clone(), on_delta).await
                    }
                };
                match result {
                    Ok(resp) => return Ok(resp),
                    Err(e) if is_transient(&e) && attempt < 2 => {
                        last_err = Some(e);
                    }
                    Err(e) => return Err(e),
                }
            }
            Err(last_err.unwrap())
        })
    }

    fn name(&self) -> &str {
        &self.config.name
    }
}

/// Probe a provider/model with a tiny request (used by the GUI "test connection").
/// Returns the model's reply text on success.
pub async fn test_connection(config: &ProviderConfig, model: &str) -> Result<String> {
    let provider = HttpModelProvider::new(config.clone(), model)?;
    let req = CompletionRequest::new(
        vec![Message::user("ping")],
        Vec::new(),
        Protocol::NativeToolCall,
    );
    let resp = provider.complete(req).await?;
    Ok(if resp.text.trim().is_empty() {
        "连接成功（模型返回空文本）".to_string()
    } else {
        resp.text
    })
}

// ---------------------------------------------------------------------------
// Persistent configuration store.
// ---------------------------------------------------------------------------

/// A JSON-file store for [`ProviderSettings`] (the GUI's source of truth).
#[derive(Debug, Clone)]
pub struct ProviderStore {
    path: PathBuf,
    settings: ProviderSettings,
}

impl ProviderStore {
    /// Open (or initialize) the store at `path`.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let settings = if path.exists() {
            let s = std::fs::read_to_string(&path)
                .map_err(|e| CoreError::from(e).with_context("reading providers.json"))?;
            serde_json::from_str(&s).unwrap_or_default()
        } else {
            ProviderSettings::default()
        };
        Ok(ProviderStore { path, settings })
    }

    /// The current settings.
    pub fn settings(&self) -> &ProviderSettings {
        &self.settings
    }

    /// Persist to disk.
    pub fn save(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).ok();
            }
        }
        let s = serde_json::to_string_pretty(&self.settings)?;
        std::fs::write(&self.path, s)
            .map_err(|e| CoreError::from(e).with_context("writing providers.json"))?;
        Ok(())
    }

    /// Add or replace a provider (by id) and persist.
    pub fn upsert(&mut self, cfg: ProviderConfig) -> Result<()> {
        if let Some(existing) = self.settings.providers.iter_mut().find(|p| p.id == cfg.id) {
            *existing = cfg;
        } else {
            self.settings.providers.push(cfg);
        }
        self.save()
    }

    /// Remove a provider (by id) and persist; clears active if it was selected.
    pub fn remove(&mut self, id: &str) -> Result<()> {
        self.settings.providers.retain(|p| p.id != id);
        if self.settings.active_provider.as_deref() == Some(id) {
            self.settings.active_provider = None;
            self.settings.active_model = None;
        }
        self.save()
    }

    /// Select the active provider + model and persist.
    pub fn set_active(&mut self, provider_id: &str, model: &str) -> Result<()> {
        let cfg = self
            .settings
            .providers
            .iter()
            .find(|p| p.id == provider_id)
            .ok_or_else(|| CoreError::not_found(format!("供应商不存在: {provider_id}")))?;
        if !cfg.models.iter().any(|m| m == model) {
            return Err(CoreError::invalid_input(format!(
                "供应商 {} 没有模型 {model}",
                cfg.name
            )));
        }
        self.settings.active_provider = Some(provider_id.to_string());
        self.settings.active_model = Some(model.to_string());
        self.save()
    }

    /// The active (provider, model).
    ///
    /// Prefers the explicit selection; but if none is set (or it's stale), it
    /// gracefully falls back to the first configured provider that has a model —
    /// so "I configured one provider+model" just works without a separate
    /// "set active" click.
    pub fn active(&self) -> Option<(&ProviderConfig, String)> {
        let cfg = self
            .settings
            .active_provider
            .as_deref()
            .and_then(|pid| self.settings.providers.iter().find(|p| p.id == pid))
            .filter(|p| !p.models.is_empty())
            // Fallback: the first provider that has at least one model.
            .or_else(|| {
                self.settings
                    .providers
                    .iter()
                    .find(|p| !p.models.is_empty())
            })?;
        let model = self
            .settings
            .active_model
            .clone()
            .filter(|m| cfg.models.iter().any(|x| x == m))
            .or_else(|| {
                cfg.default_model
                    .clone()
                    .filter(|m| cfg.models.iter().any(|x| x == m))
            })
            .or_else(|| cfg.models.first().cloned())?;
        Some((cfg, model))
    }

    /// Build a live provider from the active selection.
    pub fn build_active(&self) -> Result<HttpModelProvider> {
        let (cfg, model) = self.active().ok_or_else(|| {
            CoreError::invalid_input("尚未选择当前模型供应商，请先在“供应商”里配置并选用")
        })?;
        HttpModelProvider::new(cfg.clone(), model)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::Message;

    fn sample_req(protocol: Protocol) -> CompletionRequest {
        CompletionRequest {
            messages: vec![Message::system("你是写作助手。"), Message::user("写第一章")],
            tools: vec![ToolSpec::new(
                "write_file",
                "write a file",
                json!({ "type": "object", "properties": { "path": { "type": "string" } } }),
                vec![],
                true,
            )],
            protocol,
            sampling: SamplingParams::default(),
        }
    }

    #[test]
    fn openai_body_has_messages_and_tools() {
        let body = build_openai_body("gpt-x", 1024, &sample_req(Protocol::NativeToolCall));
        assert_eq!(body["model"], "gpt-x");
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][1]["content"], "写第一章");
        assert_eq!(body["tools"][0]["function"]["name"], "write_file");
        assert_eq!(body["tool_choice"], "auto");
    }

    #[test]
    fn openai_body_omits_tools_for_react() {
        let body = build_openai_body("m", 1024, &sample_req(Protocol::ReActText));
        assert!(body.get("tools").is_none());
    }

    #[test]
    fn parse_openai_text_and_tool_call() {
        let v = json!({
            "choices": [{
                "finish_reason": "tool_calls",
                "message": {
                    "content": "好的",
                    "tool_calls": [{
                        "id": "call_1",
                        "function": { "name": "write_file", "arguments": "{\"path\":\"a.md\"}" }
                    }]
                }
            }]
        });
        let r = parse_openai_response(&v).unwrap();
        assert_eq!(r.text, "好的");
        assert_eq!(r.finish, FinishReason::ToolUse);
        assert_eq!(r.tool_calls.len(), 1);
        assert_eq!(r.tool_calls[0].name, "write_file");
        assert_eq!(r.tool_calls[0].args["path"], "a.md");
        assert_eq!(r.tool_calls[0].id.as_str(), "call_1");
    }

    #[test]
    fn parse_openai_error() {
        let v = json!({ "error": { "message": "bad key" } });
        assert!(parse_openai_response(&v).is_err());
    }

    #[test]
    fn anthropic_body_extracts_system_and_tools() {
        let body = build_anthropic_body("claude-x", 2048, &sample_req(Protocol::NativeToolCall));
        assert_eq!(body["model"], "claude-x");
        assert_eq!(body["system"], "你是写作助手。");
        assert_eq!(body["max_tokens"], 2048);
        // First message is the user turn (system was lifted out).
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["messages"][0]["content"][0]["type"], "text");
        assert_eq!(body["tools"][0]["name"], "write_file");
    }

    #[test]
    fn anthropic_merges_consecutive_roles() {
        // assistant text + tool_use must merge into ONE assistant turn.
        let mut call = ToolCallRequest::new("write_file", json!({ "path": "a.md" }));
        call.id = ToolCallId::from_existing("tu_1");
        let msgs = vec![
            Message::user("hi"),
            Message::assistant_tool_call("我来写", call),
        ];
        let (_sys, messages) = anthropic_messages(&msgs, Protocol::NativeToolCall);
        assert_eq!(messages.len(), 2); // user, assistant
        assert_eq!(messages[1]["role"], "assistant");
        let blocks = messages[1]["content"].as_array().unwrap();
        assert_eq!(blocks.len(), 2); // text + tool_use merged
        assert_eq!(blocks[0]["type"], "text");
        assert_eq!(blocks[1]["type"], "tool_use");
    }

    #[test]
    fn react_openai_history_uses_plain_text_observations() {
        let mut call = ToolCallRequest::new("read_file", json!({ "path": "a.md" }));
        call.id = ToolCallId::from_existing("call_1");
        let result = crate::message::ToolResultRef::new(call.id.clone(), "read_file", true, false);
        let req = CompletionRequest {
            messages: vec![
                Message::assistant_tool_call("need file", call),
                Message::tool("file contents", result),
            ],
            tools: Vec::new(),
            protocol: Protocol::ReActText,
            sampling: SamplingParams::default(),
        };
        let body = build_openai_body("m", 1024, &req);
        assert_eq!(body["messages"][0]["role"], "assistant");
        assert!(body["messages"][0]["tool_calls"].is_null());
        assert!(body["messages"][0]["content"]
            .as_str()
            .unwrap()
            .contains("Action: read_file"));
        assert_eq!(body["messages"][1]["role"], "user");
        assert!(body["messages"][1]["tool_call_id"].is_null());
        assert!(body["messages"][1]["content"]
            .as_str()
            .unwrap()
            .starts_with("Observation:"));
    }

    #[test]
    fn react_anthropic_history_uses_plain_text_observations() {
        let call = ToolCallRequest::new("read_file", json!({ "path": "a.md" }));
        let result = crate::message::ToolResultRef::new(call.id.clone(), "read_file", true, false);
        let req = CompletionRequest {
            messages: vec![
                Message::assistant_tool_call("need file", call),
                Message::tool("file contents", result),
            ],
            tools: Vec::new(),
            protocol: Protocol::ReActText,
            sampling: SamplingParams::default(),
        };
        let body = build_anthropic_body("claude", 1024, &req);
        assert_eq!(body["messages"][0]["role"], "assistant");
        assert_eq!(body["messages"][0]["content"][0]["type"], "text");
        assert!(body["messages"][0]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Action: read_file"));
        assert_eq!(body["messages"][1]["role"], "user");
        assert_eq!(body["messages"][1]["content"][0]["type"], "text");
        assert!(body["messages"][1]["content"][0]["text"]
            .as_str()
            .unwrap()
            .starts_with("Observation:"));
    }

    #[test]
    fn parse_anthropic_text_and_tool_use() {
        let v = json!({
            "stop_reason": "tool_use",
            "content": [
                { "type": "text", "text": "思考中" },
                { "type": "tool_use", "id": "tu_9", "name": "read_file", "input": { "path": "x" } }
            ]
        });
        let r = parse_anthropic_response(&v).unwrap();
        assert_eq!(r.text, "思考中");
        assert_eq!(r.finish, FinishReason::ToolUse);
        assert_eq!(r.tool_calls[0].name, "read_file");
        assert_eq!(r.tool_calls[0].id.as_str(), "tu_9");
        assert_eq!(r.tool_calls[0].args["path"], "x");
    }

    #[test]
    fn gemini_body_has_system_tools_and_function_response() {
        let mut call = ToolCallRequest::new("read_file", json!({ "path": "a.md" }));
        call.id = ToolCallId::from_existing("call_1");
        let result = crate::message::ToolResultRef::new(call.id.clone(), "read_file", true, false);
        let req = CompletionRequest {
            messages: vec![
                Message::system("你是写作助手。"),
                Message::user("读文件"),
                Message::assistant_tool_call("need file", call),
                Message::tool("file contents", result),
            ],
            tools: vec![ToolSpec::new(
                "read_file",
                "read a file",
                json!({ "type": "object", "properties": { "path": { "type": "string" } } }),
                vec![],
                false,
            )],
            protocol: Protocol::NativeToolCall,
            sampling: SamplingParams::default(),
        };
        let body = build_gemini_body(1024, &req);
        assert_eq!(body["generationConfig"]["maxOutputTokens"], 1024);
        assert_eq!(
            body["systemInstruction"]["parts"][0]["text"],
            "你是写作助手。"
        );
        assert_eq!(body["contents"][0]["role"], "user");
        assert_eq!(body["contents"][1]["role"], "model");
        assert_eq!(
            body["contents"][1]["parts"][1]["functionCall"]["name"],
            "read_file"
        );
        assert_eq!(
            body["contents"][2]["parts"][0]["functionResponse"]["name"],
            "read_file"
        );
        assert_eq!(
            body["tools"][0]["functionDeclarations"][0]["name"],
            "read_file"
        );
        assert_eq!(body["toolConfig"]["functionCallingConfig"]["mode"], "AUTO");
    }

    #[test]
    fn gemini_body_omits_tools_for_react() {
        let body = build_gemini_body(1024, &sample_req(Protocol::ReActText));
        assert!(body.get("tools").is_none());
        assert!(body.get("toolConfig").is_none());
    }

    #[test]
    fn parse_gemini_text_and_function_call() {
        let v = json!({
            "candidates": [{
                "finishReason": "STOP",
                "content": {
                    "role": "model",
                    "parts": [
                        { "text": "我先读取。" },
                        { "functionCall": { "name": "read_file", "args": { "path": "a.md" } } }
                    ]
                }
            }]
        });
        let r = parse_gemini_response(&v).unwrap();
        assert_eq!(r.text, "我先读取。");
        assert_eq!(r.finish, FinishReason::ToolUse);
        assert_eq!(r.tool_calls.len(), 1);
        assert_eq!(r.tool_calls[0].name, "read_file");
        assert_eq!(r.tool_calls[0].args["path"], "a.md");
    }

    #[test]
    fn parse_gemini_text_final() {
        let v = json!({
            "candidates": [{
                "finishReason": "MAX_TOKENS",
                "content": {
                    "role": "model",
                    "parts": [{ "text": "partial" }]
                }
            }]
        });
        let r = parse_gemini_response(&v).unwrap();
        assert_eq!(r.text, "partial");
        assert_eq!(r.finish, FinishReason::Length);
        assert!(r.tool_calls.is_empty());
    }

    #[test]
    fn parse_gemini_error() {
        let v = json!({ "error": { "message": "bad key" } });
        assert!(parse_gemini_response(&v).is_err());
    }

    #[test]
    fn openai_stream_accumulates_text_and_tool_call() {
        // Three content deltas + a tool call split across deltas, then finish.
        let mut text = String::new();
        let mut tools: Vec<ToolAccum> = Vec::new();
        let mut finish = None;
        let streamed = std::cell::RefCell::new(String::new());
        {
            let sink = |d: &str| streamed.borrow_mut().push_str(d);
            for c in ["你", "好", "呀"] {
                let d = json!({ "choices": [{ "delta": { "content": c } }] });
                openai_stream_event(&d, &mut text, &mut tools, &mut finish, &sink);
            }
            // tool call name then argument fragments
            let d1 = json!({ "choices": [{ "delta": { "tool_calls": [{ "index": 0, "id": "call_1", "function": { "name": "write_file" } }] } }] });
            openai_stream_event(&d1, &mut text, &mut tools, &mut finish, &sink);
            let d2 = json!({ "choices": [{ "delta": { "tool_calls": [{ "index": 0, "function": { "arguments": "{\"path\":" } }] } }] });
            openai_stream_event(&d2, &mut text, &mut tools, &mut finish, &sink);
            let d3 = json!({ "choices": [{ "delta": { "tool_calls": [{ "index": 0, "function": { "arguments": "\"a.md\"}" } }] }, "finish_reason": "tool_calls" }] });
            openai_stream_event(&d3, &mut text, &mut tools, &mut finish, &sink);
        }
        assert_eq!(text, "你好呀");
        assert_eq!(streamed.borrow().as_str(), "你好呀");
        let resp = finish_accum(text, tools, finish);
        assert_eq!(resp.finish, FinishReason::ToolUse);
        assert_eq!(resp.tool_calls.len(), 1);
        assert_eq!(resp.tool_calls[0].name, "write_file");
        assert_eq!(resp.tool_calls[0].args["path"], "a.md");
        assert_eq!(resp.tool_calls[0].id.as_str(), "call_1");
    }

    #[test]
    fn anthropic_stream_accumulates_text_and_tool_use() {
        let mut text = String::new();
        let mut tools: Vec<ToolAccum> = Vec::new();
        let mut finish = None;
        {
            let sink = |_: &str| {};
            // text block
            anthropic_stream_event(
                &json!({ "type": "content_block_start", "index": 0, "content_block": { "type": "text" } }),
                &mut text,
                &mut tools,
                &mut finish,
                &sink,
            );
            anthropic_stream_event(
                &json!({ "type": "content_block_delta", "index": 0, "delta": { "type": "text_delta", "text": "思考" } }),
                &mut text,
                &mut tools,
                &mut finish,
                &sink,
            );
            // tool_use block
            anthropic_stream_event(
                &json!({ "type": "content_block_start", "index": 1, "content_block": { "type": "tool_use", "id": "tu_1", "name": "read_file" } }),
                &mut text,
                &mut tools,
                &mut finish,
                &sink,
            );
            anthropic_stream_event(
                &json!({ "type": "content_block_delta", "index": 1, "delta": { "type": "input_json_delta", "partial_json": "{\"path\":\"x\"}" } }),
                &mut text,
                &mut tools,
                &mut finish,
                &sink,
            );
            anthropic_stream_event(
                &json!({ "type": "message_delta", "delta": { "stop_reason": "tool_use" } }),
                &mut text,
                &mut tools,
                &mut finish,
                &sink,
            );
        }
        assert_eq!(text, "思考");
        let resp = finish_accum(text, tools, finish);
        assert_eq!(resp.finish, FinishReason::ToolUse);
        assert_eq!(resp.tool_calls.len(), 1);
        assert_eq!(resp.tool_calls[0].name, "read_file");
        assert_eq!(resp.tool_calls[0].args["path"], "x");
        assert_eq!(resp.tool_calls[0].id.as_str(), "tu_1");
    }

    #[test]
    fn gemini_stream_accumulates_text_and_function_call() {
        let mut text = String::new();
        let mut tools: Vec<ToolAccum> = Vec::new();
        let mut finish = None;
        let streamed = std::cell::RefCell::new(String::new());
        {
            let sink = |d: &str| streamed.borrow_mut().push_str(d);
            gemini_stream_event(
                &json!({
                    "candidates": [{
                        "content": {
                            "role": "model",
                            "parts": [{ "text": "先读" }]
                        }
                    }]
                }),
                &mut text,
                &mut tools,
                &mut finish,
                &sink,
            )
            .unwrap();
            gemini_stream_event(
                &json!({
                    "candidates": [{
                        "finishReason": "STOP",
                        "content": {
                            "role": "model",
                            "parts": [{
                                "functionCall": {
                                    "name": "read_file",
                                    "args": { "path": "a.md" }
                                }
                            }]
                        }
                    }]
                }),
                &mut text,
                &mut tools,
                &mut finish,
                &sink,
            )
            .unwrap();
        }

        assert_eq!(text, "先读");
        assert_eq!(streamed.borrow().as_str(), "先读");
        let resp = finish_accum(text, tools, finish);
        assert_eq!(resp.finish, FinishReason::ToolUse);
        assert_eq!(resp.tool_calls.len(), 1);
        assert_eq!(resp.tool_calls[0].name, "read_file");
        assert_eq!(resp.tool_calls[0].args["path"], "a.md");
    }

    #[test]
    fn drain_lines_keeps_partial_and_decodes_utf8() {
        // A multi-byte char split across two appends must not corrupt.
        let mut buf: Vec<u8> = Vec::new();
        let full = "data: 你好\n".as_bytes();
        buf.extend_from_slice(&full[..7]); // split mid-character
        assert!(drain_lines(&mut buf).is_empty()); // no newline yet
        buf.extend_from_slice(&full[7..]);
        let lines = drain_lines(&mut buf);
        assert_eq!(lines, vec!["data: 你好".to_string()]);
        assert_eq!(sse_data(&lines[0]), Some("你好"));
    }

    #[test]
    fn settings_round_trip_json() {
        let s = ProviderSettings {
            providers: vec![ProviderConfig {
                id: "p1".into(),
                name: "OpenAI".into(),
                protocol: ProviderProtocol::OpenAi,
                tool_mode: ProviderToolMode::Auto,
                base_url: "https://api.openai.com/v1".into(),
                api_key: "sk-x".into(),
                models: vec!["gpt-4o".into(), "gpt-4o-mini".into()],
                default_model: Some("gpt-4o".into()),
                max_tokens: Some(4096),
                sampling: SamplingParams::default(),
            }],
            active_provider: Some("p1".into()),
            active_model: Some("gpt-4o".into()),
        };
        let j = serde_json::to_string(&s).unwrap();
        let back: ProviderSettings = serde_json::from_str(&j).unwrap();
        assert_eq!(s, back);
    }

    fn temp_path(tag: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "na_providers_{}_{}.json",
            tag,
            na_common::next_id("t")
        ));
        p
    }

    fn cfg(id: &str) -> ProviderConfig {
        ProviderConfig {
            id: id.into(),
            name: format!("provider-{id}"),
            protocol: ProviderProtocol::OpenAi,
            tool_mode: ProviderToolMode::Auto,
            base_url: "https://example.test/v1".into(),
            api_key: "key".into(),
            models: vec!["m1".into(), "m2".into()],
            default_model: None,
            max_tokens: None,
            sampling: SamplingParams::default(),
        }
    }

    #[test]
    fn store_crud_and_active() {
        let path = temp_path("crud");
        let mut store = ProviderStore::open(&path).unwrap();
        store.upsert(cfg("a")).unwrap();
        store.upsert(cfg("b")).unwrap();
        assert_eq!(store.settings().providers.len(), 2);

        // Reopen persists.
        let store2 = ProviderStore::open(&path).unwrap();
        assert_eq!(store2.settings().providers.len(), 2);

        // set_active validates model membership.
        let mut store = store2;
        assert!(store.set_active("a", "nope").is_err());
        store.set_active("a", "m2").unwrap();
        let (active_cfg, model) = store.active().unwrap();
        assert_eq!(active_cfg.id, "a");
        assert_eq!(model, "m2");

        // remove clears the explicit selection but falls back to remaining provider b.
        store.remove("a").unwrap();
        assert!(store.settings().active_provider.is_none());
        assert_eq!(store.settings().providers.len(), 1);
        let (fallback_cfg, _m) = store.active().unwrap();
        assert_eq!(fallback_cfg.id, "b");
    }

    #[test]
    fn active_falls_back_without_explicit_selection() {
        let path = temp_path("fallback");
        let mut store = ProviderStore::open(&path).unwrap();
        store.upsert(cfg("only")).unwrap();
        // Never called set_active — but a configured provider+model should still work.
        let (c, m) = store
            .active()
            .expect("should fall back to the sole provider");
        assert_eq!(c.id, "only");
        assert_eq!(m, "m1");
        assert!(store.build_active().is_ok());
    }

    #[test]
    fn build_active_errors_when_no_providers() {
        let path = temp_path("empty");
        let store = ProviderStore::open(&path).unwrap();
        assert!(store.build_active().is_err());
    }

    #[test]
    fn provider_config_defaults_tool_mode_for_old_json() {
        let raw = r#"{
            "id": "old",
            "name": "Old Provider",
            "protocol": "open_ai",
            "base_url": "https://example.test/v1",
            "api_key": "key",
            "models": ["m"]
        }"#;
        let cfg: ProviderConfig = serde_json::from_str(raw).unwrap();
        assert_eq!(cfg.tool_mode, ProviderToolMode::Auto);
        assert_eq!(cfg.agent_protocol(), Protocol::ReActText);
    }

    #[test]
    fn tool_mode_selects_agent_protocol() {
        let mut cfg = cfg("modes");
        cfg.protocol = ProviderProtocol::OpenAi;
        cfg.tool_mode = ProviderToolMode::Auto;
        assert_eq!(cfg.agent_protocol(), Protocol::ReActText);

        cfg.protocol = ProviderProtocol::Anthropic;
        assert_eq!(cfg.agent_protocol(), Protocol::NativeToolCall);

        cfg.protocol = ProviderProtocol::Gemini;
        assert_eq!(cfg.agent_protocol(), Protocol::NativeToolCall);

        cfg.tool_mode = ProviderToolMode::Native;
        assert_eq!(cfg.agent_protocol(), Protocol::NativeToolCall);

        cfg.tool_mode = ProviderToolMode::Text;
        assert_eq!(cfg.agent_protocol(), Protocol::ReActText);
    }

    #[test]
    fn native_tool_http_error_mentions_compatibility_mode() {
        let msg = provider_status_message(
            "P",
            StatusCode::INTERNAL_SERVER_ERROR,
            "bad_response_status_code",
            &sample_req(Protocol::NativeToolCall),
        );
        assert!(msg.contains("文本工具"));

        let plain = provider_status_message(
            "P",
            StatusCode::INTERNAL_SERVER_ERROR,
            "bad_response_status_code",
            &sample_req(Protocol::ReActText),
        );
        assert!(!plain.contains("文本工具"));
    }

    #[test]
    fn stream_status_failures_are_retryable() {
        assert!(should_retry_without_stream(
            StatusCode::BAD_REQUEST,
            "stream is not supported"
        ));
        assert!(should_retry_without_stream(
            StatusCode::INTERNAL_SERVER_ERROR,
            "bad_response_status_code"
        ));
        assert!(!should_retry_without_stream(
            StatusCode::UNAUTHORIZED,
            "invalid api key"
        ));
    }
}
