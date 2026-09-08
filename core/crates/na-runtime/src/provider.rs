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

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use reqwest::StatusCode;
use serde::{Deserialize, Serialize};

use na_common::{json, CoreError, Json, Result, ToolCallId};
use na_tools::ToolSpec;

use crate::context::estimate_tokens;
use crate::message::{Message, Role, ToolCallRequest};
use crate::model::{
    BoxFuture, CompletionRequest, CompletionResponse, FinishReason, ModelProvider, Protocol,
    SamplingParams, UsageMetadata,
};
use crate::react::{parse_react, ReActStep};
use crate::session::atomic_write;

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
    if uses_native_tools(req) && is_tool_compatibility_error(&CoreError::model(msg.clone())) {
        msg.push_str(
            "。该模型可能不支持原生工具调用；自动兼容模式会尝试回退到文本工具，也可以在“供应商”中明确选择“文本工具”。",
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
        .strip_prefix("models/")
        .unwrap_or(model)
        .split('/')
        .map(|part| part.replace(':', "%3A"))
        .collect::<Vec<_>>()
        .join("/")
}

fn gemini_model_resource(model: &str) -> String {
    format!("models/{}", gemini_model_path(model))
}

/// Build a Gemini REST endpoint while accepting both the documented host-only
/// base URL and a custom base URL that already ends in `/v1beta`.
fn gemini_endpoint(base_url: &str, path: &str) -> String {
    let base = base_url.trim_end_matches('/');
    let base = base.strip_suffix("/v1beta").unwrap_or(base);
    format!("{base}/v1beta/{path}")
}

const GEMINI_CACHE_TTL: Duration = Duration::from_secs(60 * 60);
const GEMINI_CACHE_RETRY: Duration = Duration::from_secs(60);

/// Gemini's minimum cache size varies by model family. The conservative values
/// below avoid repeatedly asking the API to create a cache that it will reject.
fn gemini_cache_min_tokens(model: &str) -> usize {
    let lower = model.to_ascii_lowercase();
    if lower.contains("2.5-pro") {
        4_096
    } else if lower.contains("1.5-pro") {
        2_048
    } else {
        1_024
    }
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct GeminiCacheKey {
    base_url: String,
    model: String,
    api_key_hash: u64,
    system_hash: u64,
    prefix_hash: u64,
    prefix_len: usize,
}

#[derive(Debug)]
enum GeminiCacheRecord {
    Active {
        resource_name: String,
        expires_at: Instant,
    },
    /// Cache creation is best-effort. Remember a failed endpoint briefly so a
    /// long agent run does not issue one failing cache request per model turn.
    Unavailable { retry_at: Instant },
}

#[derive(Debug, Clone)]
struct GeminiCacheHandle {
    key: GeminiCacheKey,
    resource_name: String,
    prefix_len: usize,
}

#[derive(Debug, Clone)]
struct GeminiCachePlan {
    key: GeminiCacheKey,
    model_resource: String,
    system: String,
    prefix_contents: Vec<Json>,
    prefix_len: usize,
}

static GEMINI_CACHE_REGISTRY: OnceLock<Mutex<HashMap<GeminiCacheKey, GeminiCacheRecord>>> =
    OnceLock::new();

fn gemini_cache_registry() -> &'static Mutex<HashMap<GeminiCacheKey, GeminiCacheRecord>> {
    GEMINI_CACHE_REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn hash_text(value: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
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
    let mut index = 0usize;
    while index < msgs.len() {
        let m = &msgs[index];
        if protocol == Protocol::NativeToolCall
            && m.role == Role::Assistant
            && m.tool_call.is_some()
        {
            // OpenAI requires one assistant message containing all calls from a
            // turn, followed by one tool message per result. The internal
            // transcript stores calls as separate messages for simple UI
            // rendering, so merge adjacent calls at the wire boundary.
            let mut text = String::new();
            let mut calls = Vec::new();
            while index < msgs.len()
                && msgs[index].role == Role::Assistant
                && msgs[index].tool_call.is_some()
            {
                let current = &msgs[index];
                if text.is_empty() && !current.content.is_empty() {
                    text.push_str(&current.content);
                }
                let Some(call) = current.tool_call.as_ref() else {
                    break;
                };
                calls.push(json!({
                    "id": call.id.as_str(),
                    "type": "function",
                    "function": { "name": call.name, "arguments": call.args.to_string() }
                }));
                index += 1;
            }
            out.push(json!({
                "role": "assistant",
                "content": if text.is_empty() { Json::Null } else { json!(text) },
                "tool_calls": calls,
            }));
            continue;
        }
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
        index += 1;
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
    if let Some(tcs) = msg
        .get("tool_calls")
        .and_then(Json::as_array)
        .filter(|calls| !calls.is_empty())
    {
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
            let args = match func.and_then(|f| f.get("arguments")) {
                Some(Json::String(raw)) => {
                    serde_json::from_str::<Json>(raw).unwrap_or_else(|_| json!({ "_raw": raw }))
                }
                // A number of OpenAI-compatible gateways return the already
                // decoded object instead of the documented JSON string.
                Some(value) if value.is_object() => value.clone(),
                _ => json!({}),
            };
            if !name.is_empty() {
                let cid = if id.is_empty() {
                    ToolCallId::new()
                } else {
                    ToolCallId::from_existing(id)
                };
                tool_calls.push(ToolCallRequest::with_id(cid, name, args));
            }
        }
    } else if let Some(function) = msg.get("function_call") {
        // Older OpenAI-compatible gateways still emit the pre-0613
        // `function_call` shape. Treat it as one native tool call so the
        // agent remains compatible with both generations of the API.
        let name = function.get("name").and_then(Json::as_str).unwrap_or("");
        if !name.is_empty() {
            let args = match function.get("arguments") {
                Some(Json::String(raw)) => {
                    serde_json::from_str::<Json>(raw).unwrap_or_else(|_| json!({ "_raw": raw }))
                }
                Some(value) if value.is_object() => value.clone(),
                _ => json!({}),
            };
            tool_calls.push(ToolCallRequest::new(name, args));
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
        usage: None,
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
                        json!({
                            "type": "tool_result",
                            "tool_use_id": id,
                            "content": m.content,
                            "is_error": m.tool_result.as_ref().map(|result| !result.ok).unwrap_or(false),
                        }),
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
        usage: None,
    })
}

fn gemini_function_declaration(spec: &ToolSpec) -> Json {
    json!({
        "name": spec.name,
        "description": spec.description,
        "parameters": gemini_schema(&spec.input_schema),
    })
}

/// Gemini accepts an OpenAPI schema subset for function declarations. The
/// internal tool schemas intentionally use full JSON Schema (for local
/// validation), so sending them verbatim makes some Gemini models reject the
/// entire request with an opaque "tool is incompatible" error. Keep the
/// provider mapping conservative and strip keywords Gemini does not implement.
fn gemini_schema(schema: &Json) -> Json {
    let Some(object) = schema.as_object() else {
        return json!({ "type": "OBJECT" });
    };

    let mut out = serde_json::Map::new();
    for key in [
        "format",
        "description",
        "enum",
        "minItems",
        "maxItems",
        "minProperties",
        "maxProperties",
    ] {
        if let Some(value) = object.get(key) {
            out.insert(key.to_string(), value.clone());
        }
    }
    if let Some(nullable) = object.get("nullable").and_then(Json::as_bool) {
        out.insert("nullable".to_string(), json!(nullable));
    }

    // JSON Schema permits a union in `type`; Gemini expects one scalar type
    // plus an optional nullable flag. Prefer the first non-null member.
    match object.get("type") {
        Some(Json::String(value)) => {
            if let Some(mapped) = gemini_schema_type(value) {
                out.insert("type".to_string(), json!(mapped));
            }
        }
        Some(Json::Array(types)) => {
            if types.iter().any(|value| {
                value
                    .as_str()
                    .map(|value| value.eq_ignore_ascii_case("null"))
                    == Some(true)
            }) {
                out.insert("nullable".to_string(), json!(true));
            }
            if let Some(value) = types
                .iter()
                .filter_map(Json::as_str)
                .find_map(gemini_schema_type)
            {
                out.insert("type".to_string(), json!(value));
            }
        }
        _ => {}
    }

    // JSON Schema allows an enum-only property to omit `type`. Infer the
    // scalar type so Gemini does not receive an OBJECT parameter with string
    // enum values (a common source of "tool incompatible" errors).
    if !out.contains_key("type") {
        if let Some(inferred) = object.get("enum").and_then(gemini_enum_type) {
            out.insert("type".to_string(), json!(inferred));
        }
    }

    // Collapse simple unions used by generated JSON Schema documents. Gemini
    // has no `oneOf`/`anyOf` in function declarations; local validation still
    // uses the original schema before a call is executed.
    if !out.contains_key("type") {
        for key in ["oneOf", "anyOf"] {
            if let Some(Json::Array(variants)) = object.get(key) {
                for variant in variants {
                    let mapped = gemini_schema(variant);
                    if mapped.get("nullable").and_then(Json::as_bool) == Some(true) {
                        out.insert("nullable".to_string(), json!(true));
                    }
                    if let Some(mapped_type) = mapped.get("type") {
                        // A `null` branch maps to a placeholder object. Prefer
                        // the first actual value type in a nullable union.
                        let is_null_variant = variant
                            .get("type")
                            .and_then(Json::as_str)
                            .map(|value| value.eq_ignore_ascii_case("null"))
                            .unwrap_or(false);
                        if !is_null_variant || mapped_type.as_str() != Some("OBJECT") {
                            out.insert("type".to_string(), mapped_type.clone());
                            break;
                        }
                    }
                }
                break;
            }
        }
    }

    if let Some(properties) = object.get("properties").and_then(Json::as_object) {
        let mapped = properties
            .iter()
            .map(|(name, value)| (name.clone(), gemini_schema(value)))
            .collect();
        out.insert("properties".to_string(), Json::Object(mapped));
    }
    if let Some(required) = object.get("required").and_then(Json::as_array) {
        let names: Vec<Json> = required
            .iter()
            .filter(|value| value.as_str().is_some())
            .cloned()
            .collect();
        if !names.is_empty() {
            out.insert("required".to_string(), Json::Array(names));
        }
    }
    if let Some(items) = object.get("items") {
        out.insert("items".to_string(), gemini_schema(items));
    }

    // A malformed/non-object tool schema should still produce a valid Gemini
    // declaration. Local validation remains responsible for the real schema.
    if !out.contains_key("type") {
        out.insert("type".to_string(), json!("OBJECT"));
    }
    Json::Object(out)
}

/// Gemini's REST schema enum is uppercase, while the local JSON Schema uses
/// lowercase names. Unknown and `null` types are omitted so they cannot make a
/// function declaration invalid; nullable unions are handled by the caller.
fn gemini_schema_type(value: &str) -> Option<&'static str> {
    match value.to_ascii_lowercase().as_str() {
        "string" => Some("STRING"),
        "number" => Some("NUMBER"),
        "integer" => Some("INTEGER"),
        "boolean" => Some("BOOLEAN"),
        "array" => Some("ARRAY"),
        "object" => Some("OBJECT"),
        _ => None,
    }
}

fn gemini_enum_type(value: &Json) -> Option<&'static str> {
    let values = value.as_array()?;
    if values.is_empty() {
        return None;
    }
    if values.iter().all(Json::is_string) {
        Some("STRING")
    } else if values.iter().all(Json::is_boolean) {
        Some("BOOLEAN")
    } else if values.iter().all(|value| value.is_i64() || value.is_u64()) {
        Some("INTEGER")
    } else if values.iter().all(Json::is_number) {
        Some("NUMBER")
    } else {
        None
    }
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

/// Gemini expects one content turn per role transition. In particular, all
/// responses to a parallel function-call batch must be parts of the same
/// `user` turn, not a series of adjacent user messages.
fn gemini_push_parts(contents: &mut Vec<Json>, role: &'static str, parts: Vec<Json>) {
    if parts.is_empty() {
        return;
    }
    if let Some(last) = contents.last_mut() {
        if last.get("role").and_then(Json::as_str) == Some(role) {
            if let Some(existing) = last.get_mut("parts").and_then(Json::as_array_mut) {
                existing.extend(parts);
                return;
            }
        }
    }
    contents.push(json!({ "role": role, "parts": parts }));
}

/// Parse Gemini's optional token accounting block. The API uses camelCase
/// keys, while the internal model keeps Rust-style names for callers.
fn parse_gemini_usage(v: &Json) -> Option<UsageMetadata> {
    let usage = v.get("usageMetadata")?;
    let value = |key: &str| {
        usage
            .get(key)
            .and_then(Json::as_u64)
            .and_then(|n| u32::try_from(n).ok())
    };
    let parsed = UsageMetadata {
        prompt_token_count: value("promptTokenCount"),
        response_token_count: value("responseTokenCount").or_else(|| value("candidatesTokenCount")),
        total_token_count: value("totalTokenCount"),
        cached_content_token_count: value("cachedContentTokenCount"),
    };
    if parsed.prompt_token_count.is_none()
        && parsed.response_token_count.is_none()
        && parsed.total_token_count.is_none()
        && parsed.cached_content_token_count.is_none()
    {
        None
    } else {
        Some(parsed)
    }
}

fn gemini_contents(msgs: &[Message], protocol: Protocol) -> (Option<String>, Vec<Json>) {
    let mut system_parts: Vec<String> = Vec::new();
    let mut contents: Vec<Json> = Vec::new();

    let mut index = 0usize;
    while index < msgs.len() {
        let m = &msgs[index];
        if protocol == Protocol::NativeToolCall
            && m.role == Role::Assistant
            && m.tool_call.is_some()
        {
            // Gemini models expect all function calls emitted in one model
            // turn to share one `content.parts` array. Merge the internal
            // per-call transcript messages before sending them.
            let mut parts = Vec::new();
            while index < msgs.len()
                && msgs[index].role == Role::Assistant
                && msgs[index].tool_call.is_some()
            {
                let current = &msgs[index];
                if parts.is_empty() && !current.content.trim().is_empty() {
                    parts.push(gemini_text_part(&current.content));
                }
                let Some(call) = current.tool_call.as_ref() else {
                    break;
                };
                parts.push(gemini_function_call_part(call));
                index += 1;
            }
            gemini_push_parts(&mut contents, "model", parts);
            continue;
        }
        match m.role {
            Role::System => system_parts.push(m.content.clone()),
            Role::User => {
                gemini_push_parts(&mut contents, "user", vec![gemini_text_part(&m.content)])
            }
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
                gemini_push_parts(&mut contents, "model", parts);
            }
            Role::Tool => {
                let part = if protocol == Protocol::ReActText {
                    gemini_text_part(&react_observation_text(&m.content))
                } else {
                    gemini_function_response_part(m)
                };
                gemini_push_parts(&mut contents, "user", vec![part]);
            }
        }
        index += 1;
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

fn build_gemini_cache_body(plan: &GeminiCachePlan) -> Json {
    let mut body = json!({
        "model": plan.model_resource,
        "displayName": "novel-generate-agent",
        "systemInstruction": {
            "parts": [{ "text": plan.system }]
        },
        "ttl": format!("{}s", GEMINI_CACHE_TTL.as_secs()),
    });
    if !plan.prefix_contents.is_empty() {
        body["contents"] = json!(plan.prefix_contents);
    }
    body
}

/// Attach a cached context to a normal generation body. The cached prefix is
/// removed from `contents`; sending it again would count the same tokens twice
/// and defeats the point of explicit context caching.
fn apply_gemini_cache(body: &mut Json, handle: &GeminiCacheHandle) {
    body["cachedContent"] = json!(handle.resource_name);
    if handle.prefix_len > 0 {
        if let Some(contents) = body.get("contents").and_then(Json::as_array) {
            body["contents"] = json!(contents
                .iter()
                .skip(handle.prefix_len)
                .cloned()
                .collect::<Vec<_>>());
        }
    }
    if let Some(object) = body.as_object_mut() {
        object.remove("systemInstruction");
    }
}

fn is_gemini_cache_error(status: StatusCode, body: &str) -> bool {
    if !matches!(status.as_u16(), 400 | 404 | 409 | 410 | 422) {
        return false;
    }
    let lower = body.to_ascii_lowercase();
    lower.contains("cachedcontent") || lower.contains("cached content") || lower.contains("cache")
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
        usage: parse_gemini_usage(v),
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

fn parse_sse_json(data: &str) -> Result<Json> {
    serde_json::from_str(data)
        .map_err(|error| CoreError::model(format!("流式响应包含无效 JSON: {error}")))
}

fn take_trailing_line(buf: &mut Vec<u8>) -> Option<String> {
    if buf.is_empty() {
        return None;
    }
    let bytes = std::mem::take(buf);
    Some(
        String::from_utf8_lossy(&bytes)
            .trim_end_matches(['\n', '\r'])
            .to_string(),
    )
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
        if let Some(function) = delta.get("function_call") {
            if tools.is_empty() {
                tools.push(ToolAccum::default());
            }
            let slot = &mut tools[0];
            if let Some(name) = function.get("name").and_then(Json::as_str) {
                slot.name.push_str(name);
            }
            if let Some(args) = function.get("arguments").and_then(Json::as_str) {
                slot.args.push_str(args);
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
#[allow(dead_code)]
fn gemini_stream_event(
    d: &Json,
    text: &mut String,
    tools: &mut Vec<ToolAccum>,
    finish: &mut Option<FinishReason>,
    on_delta: &dyn Fn(&str),
) -> Result<()> {
    gemini_stream_event_with_usage(d, text, tools, finish, on_delta, &mut None)
}

/// Variant of [`gemini_stream_event`] that retains usage metadata from the
/// final SSE chunk for the completed response.
fn gemini_stream_event_with_usage(
    d: &Json,
    text: &mut String,
    tools: &mut Vec<ToolAccum>,
    finish: &mut Option<FinishReason>,
    on_delta: &dyn Fn(&str),
    usage: &mut Option<UsageMetadata>,
) -> Result<()> {
    if let Some(err) = d.get("error") {
        let msg = err
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("unknown provider error");
        return Err(CoreError::model(format!("provider error: {msg}")));
    }

    if let Some(parsed) = parse_gemini_usage(d) {
        *usage = Some(parsed);
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
) -> Result<CompletionResponse> {
    finish_accum_with_usage(text, tools, finish, None)
}

fn finish_accum_with_usage(
    text: String,
    tools: Vec<ToolAccum>,
    finish: Option<FinishReason>,
    usage: Option<UsageMetadata>,
) -> Result<CompletionResponse> {
    let mut tool_calls = Vec::new();
    for t in tools {
        if t.name.is_empty() {
            continue;
        }
        let args = if t.args.trim().is_empty() {
            json!({})
        } else {
            serde_json::from_str::<Json>(&t.args).map_err(|error| {
                CoreError::model(format!(
                    "供应商返回了无效的工具参数 JSON（工具 {:?}）: {error}",
                    t.name
                ))
            })?
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
    Ok(CompletionResponse {
        text,
        tool_calls,
        finish,
        usage,
    })
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
            .user_agent(concat!("novel-generate-agent/", env!("CARGO_PKG_VERSION")))
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

    fn gemini_cache_plan(&self, req: &CompletionRequest) -> Option<GeminiCachePlan> {
        // A cached prefix must never end inside a function-call round trip.
        // Gemini validates the cached transcript as a whole, and replaying a
        // model `functionCall` without its matching `functionResponse` causes
        // intermittent INVALID_ARGUMENT/tool compatibility failures. Once a
        // run has entered tool mode, use the ordinary request body instead.
        if req
            .messages
            .iter()
            .any(|message| message.tool_call.is_some() || message.tool_result.is_some())
        {
            return None;
        }
        let (system, contents) = gemini_contents(&req.messages, req.protocol);
        let system = system?.trim().to_string();
        if system.is_empty() {
            return None;
        }

        // System instructions and the oldest transcript turns are stable across
        // an agent loop. Select the shortest prefix that reaches the provider's
        // cache floor; newer turns remain dynamic and are sent in the request.
        let mut prefix_contents = Vec::new();
        let mut prefix_len = 0usize;
        let mut estimated = estimate_tokens(&system);
        let min_tokens = gemini_cache_min_tokens(&self.model);
        for (index, content) in contents.iter().enumerate() {
            if estimated >= min_tokens {
                break;
            }
            estimated = estimated.saturating_add(estimate_tokens(
                &serde_json::to_string(content).unwrap_or_default(),
            ));
            prefix_contents.push(content.clone());
            prefix_len = index + 1;
        }
        if estimated < min_tokens {
            return None;
        }

        // The cachedContents endpoint expects at least one content turn. Keep
        // the first real turn in the cache even when the system instruction by
        // itself already reaches the token floor; it is removed from the
        // generation suffix when a later turn is available.
        if prefix_contents.is_empty() {
            let first = contents.first()?.clone();
            prefix_contents.push(first);
            prefix_len = 1;
        }

        let prefix_json = serde_json::to_string(&prefix_contents).ok()?;
        let key = GeminiCacheKey {
            base_url: self.config.base_url.trim_end_matches('/').to_string(),
            model: gemini_model_resource(&self.model),
            api_key_hash: hash_text(&self.config.api_key),
            system_hash: hash_text(&system),
            prefix_hash: hash_text(&prefix_json),
            prefix_len,
        };
        Some(GeminiCachePlan {
            key,
            model_resource: gemini_model_resource(&self.model),
            system,
            prefix_contents,
            prefix_len,
        })
    }

    async fn ensure_gemini_cache(&self, plan: &GeminiCachePlan) -> Option<GeminiCacheHandle> {
        let now = Instant::now();
        if let Ok(mut registry) = gemini_cache_registry().lock() {
            match registry.get(&plan.key) {
                Some(GeminiCacheRecord::Active {
                    resource_name,
                    expires_at,
                }) if *expires_at > now => {
                    return Some(GeminiCacheHandle {
                        key: plan.key.clone(),
                        resource_name: resource_name.clone(),
                        prefix_len: plan.prefix_len,
                    });
                }
                Some(GeminiCacheRecord::Unavailable { retry_at }) if *retry_at > now => {
                    return None;
                }
                _ => {
                    registry.remove(&plan.key);
                }
            }
        }

        let response = match self
            .client
            .post(gemini_endpoint(&self.config.base_url, "cachedContents"))
            .header("x-goog-api-key", &self.config.api_key)
            .json(&build_gemini_cache_body(plan))
            .send()
            .await
        {
            Ok(response) => response,
            Err(_) => {
                self.mark_gemini_cache_unavailable(&plan.key);
                return None;
            }
        };
        let status = response.status();
        let body = match response.text().await {
            Ok(body) => body,
            Err(_) => {
                self.mark_gemini_cache_unavailable(&plan.key);
                return None;
            }
        };
        if !status.is_success() {
            self.mark_gemini_cache_unavailable(&plan.key);
            return None;
        }
        let resource_name = match serde_json::from_str::<Json>(&body)
            .ok()
            .and_then(|value| value.get("name").and_then(Json::as_str).map(str::to_string))
        {
            Some(name) => name,
            None => {
                self.mark_gemini_cache_unavailable(&plan.key);
                return None;
            }
        };
        if let Ok(mut registry) = gemini_cache_registry().lock() {
            registry.insert(
                plan.key.clone(),
                GeminiCacheRecord::Active {
                    resource_name: resource_name.clone(),
                    expires_at: Instant::now() + GEMINI_CACHE_TTL,
                },
            );
        }
        Some(GeminiCacheHandle {
            key: plan.key.clone(),
            resource_name,
            prefix_len: plan.prefix_len,
        })
    }

    fn invalidate_gemini_cache(&self, handle: &GeminiCacheHandle) {
        if let Ok(mut registry) = gemini_cache_registry().lock() {
            registry.remove(&handle.key);
        }
    }

    fn mark_gemini_cache_unavailable(&self, key: &GeminiCacheKey) {
        if let Ok(mut registry) = gemini_cache_registry().lock() {
            registry.insert(
                key.clone(),
                GeminiCacheRecord::Unavailable {
                    retry_at: Instant::now() + GEMINI_CACHE_RETRY,
                },
            );
        }
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
        let plan = self.gemini_cache_plan(&req);
        let cache = match plan.as_ref() {
            Some(plan) => self.ensure_gemini_cache(plan).await,
            None => None,
        };
        let mut body = build_gemini_body(self.max_tokens, &req);
        if let Some(handle) = &cache {
            let has_dynamic_suffix = body
                .get("contents")
                .and_then(Json::as_array)
                .map(|contents| handle.prefix_len < contents.len())
                .unwrap_or(false);
            if has_dynamic_suffix {
                apply_gemini_cache(&mut body, handle);
            }
        }

        let url = gemini_endpoint(
            &self.config.base_url,
            &format!("models/{}:generateContent", gemini_model_path(&self.model)),
        );
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
            if let Some(handle) = &cache {
                if is_gemini_cache_error(status, &txt) {
                    self.invalidate_gemini_cache(handle);
                    // A stale/unsupported cache must not make an otherwise
                    // valid generation fail. Retry once with the plain body.
                    return self.call_gemini_without_cache(req).await;
                }
            }
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

    async fn call_gemini_without_cache(
        &self,
        req: CompletionRequest,
    ) -> Result<CompletionResponse> {
        let url = gemini_endpoint(
            &self.config.base_url,
            &format!("models/{}:generateContent", gemini_model_path(&self.model)),
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
                    if data.is_empty() {
                        continue;
                    }
                    let d = parse_sse_json(data)?;
                    saw_event = true;
                    openai_stream_event(&d, &mut text, &mut tools, &mut finish, on_delta);
                }
            }
            if done {
                break;
            }
        }
        if !done {
            if let Some(line) = take_trailing_line(&mut buf) {
                if let Some(data) = sse_data(&line) {
                    if data == "[DONE]" {
                        saw_event = true;
                    } else if !data.is_empty() {
                        let event = parse_sse_json(data)?;
                        saw_event = true;
                        openai_stream_event(&event, &mut text, &mut tools, &mut finish, on_delta);
                    }
                }
            }
        }
        // Some endpoints ignore `stream: true` and return a normal JSON body.
        // If we never parsed an SSE event, fall back to a plain completion.
        if !saw_event {
            return self.call_openai(req).await;
        }
        finish_accum(text, tools, finish)
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
                    if data == "[DONE]" {
                        saw_event = true;
                        continue;
                    }
                    if data.is_empty() {
                        continue;
                    }
                    let d = parse_sse_json(data)?;
                    saw_event = true;
                    anthropic_stream_event(&d, &mut text, &mut tools, &mut finish, on_delta);
                }
            }
        }
        if let Some(line) = take_trailing_line(&mut buf) {
            if let Some(data) = sse_data(&line) {
                if data == "[DONE]" {
                    saw_event = true;
                } else if !data.is_empty() {
                    let event = parse_sse_json(data)?;
                    saw_event = true;
                    anthropic_stream_event(&event, &mut text, &mut tools, &mut finish, on_delta);
                }
            }
        }
        // Some endpoints ignore `stream: true` and return a normal JSON body.
        // If we never parsed an SSE event, fall back to a plain completion.
        if !saw_event {
            return self.call_anthropic(req).await;
        }
        finish_accum(text, tools, finish)
    }

    async fn call_gemini_stream(
        &self,
        req: CompletionRequest,
        on_delta: &(dyn Fn(&str) + Send + Sync),
    ) -> Result<CompletionResponse> {
        let plan = self.gemini_cache_plan(&req);
        let cache = match plan.as_ref() {
            Some(plan) => self.ensure_gemini_cache(plan).await,
            None => None,
        };
        let mut body = build_gemini_body(self.max_tokens, &req);
        if let Some(handle) = &cache {
            let has_dynamic_suffix = body
                .get("contents")
                .and_then(Json::as_array)
                .map(|contents| handle.prefix_len < contents.len())
                .unwrap_or(false);
            if has_dynamic_suffix {
                apply_gemini_cache(&mut body, handle);
            }
        }
        let url = format!(
            "{}?alt=sse",
            gemini_endpoint(
                &self.config.base_url,
                &format!(
                    "models/{}:streamGenerateContent",
                    gemini_model_path(&self.model)
                ),
            )
        );
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
            if let Some(handle) = &cache {
                if is_gemini_cache_error(status, &txt) {
                    self.mark_gemini_cache_unavailable(&handle.key);
                    // Retry without the cache. This preserves correctness for
                    // endpoints that expose generateContent but not
                    // cachedContents, while still streaming the answer.
                    let response = self.call_gemini(req).await?;
                    if !response.text.is_empty() {
                        on_delta(&response.text);
                    }
                    return Ok(response);
                }
            }
            if should_retry_without_stream(status, &txt) {
                let response = self.call_gemini(req).await?;
                if !response.text.is_empty() {
                    on_delta(&response.text);
                }
                return Ok(response);
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
        let mut usage: Option<UsageMetadata> = None;
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
                    if data.is_empty() {
                        continue;
                    }
                    let d = parse_sse_json(data)?;
                    saw_event = true;
                    gemini_stream_event_with_usage(
                        &d,
                        &mut text,
                        &mut tools,
                        &mut finish,
                        on_delta,
                        &mut usage,
                    )?;
                }
            }
            if done {
                break;
            }
        }
        if !done {
            if let Some(line) = take_trailing_line(&mut buf) {
                if let Some(data) = sse_data(&line) {
                    if data == "[DONE]" {
                        saw_event = true;
                    } else if !data.is_empty() {
                        let event = parse_sse_json(data)?;
                        saw_event = true;
                        gemini_stream_event_with_usage(
                            &event,
                            &mut text,
                            &mut tools,
                            &mut finish,
                            on_delta,
                            &mut usage,
                        )?;
                    }
                }
            }
        }
        // If an endpoint ignores SSE and returns a normal/non-SSE body, retry
        // through the plain Gemini completion path rather than returning empty.
        if !saw_event {
            let response = self.call_gemini(req).await?;
            if !response.text.is_empty() {
                on_delta(&response.text);
            }
            return Ok(response);
        }
        finish_accum_with_usage(text, tools, finish, usage)
    }

    async fn complete_once(&self, request: CompletionRequest) -> Result<CompletionResponse> {
        validate_generation_messages(&request)?;
        let response = match self.config.protocol {
            ProviderProtocol::OpenAi => self.call_openai(request.clone()).await,
            ProviderProtocol::Anthropic => self.call_anthropic(request.clone()).await,
            ProviderProtocol::Gemini => self.call_gemini(request.clone()).await,
        }?;
        if request.protocol == Protocol::NativeToolCall
            && !request.tools.is_empty()
            && response.tool_calls.is_empty()
        {
            Ok(adapt_react_fallback(response))
        } else {
            Ok(response)
        }
    }

    async fn complete_streaming_once(
        &self,
        request: CompletionRequest,
        on_delta: &(dyn Fn(&str) + Send + Sync),
    ) -> Result<CompletionResponse> {
        validate_generation_messages(&request)?;
        let response = match self.config.protocol {
            ProviderProtocol::OpenAi => self.call_openai_stream(request.clone(), on_delta).await,
            ProviderProtocol::Anthropic => {
                self.call_anthropic_stream(request.clone(), on_delta).await
            }
            ProviderProtocol::Gemini => self.call_gemini_stream(request.clone(), on_delta).await,
        }?;
        if request.protocol == Protocol::NativeToolCall
            && !request.tools.is_empty()
            && response.tool_calls.is_empty()
        {
            Ok(adapt_react_fallback(response))
        } else {
            Ok(response)
        }
    }
}

/// Convert a text-protocol retry back into the structured response expected by
/// the already-running native agent loop. This lets Auto mode recover from a
/// provider that advertises chat completions but rejects native tools without
/// restarting the session or treating the ReAct text as a final answer.
fn adapt_react_fallback(response: CompletionResponse) -> CompletionResponse {
    let usage = response.usage.clone();
    match parse_react(&response.text) {
        Ok(ReActStep::Action {
            thought,
            tool,
            input,
        }) => CompletionResponse {
            text: thought.unwrap_or_default(),
            tool_calls: vec![ToolCallRequest::new(tool, input)],
            finish: FinishReason::ToolUse,
            usage,
        },
        Ok(ReActStep::Final { answer, .. }) => CompletionResponse {
            text: answer,
            tool_calls: Vec::new(),
            finish: FinishReason::Stop,
            usage,
        },
        Err(_) => response,
    }
}

/// Returns true for transient network errors worth retrying (connection refused,
/// timeout, DNS failure). API-level errors (4xx/5xx) are not transient.
fn validate_generation_messages(request: &CompletionRequest) -> Result<()> {
    if !request.messages.iter().any(|message| {
        !message.is_system()
            && (!message.content.trim().is_empty()
                || message.tool_call.is_some()
                || message.tool_result.is_some())
    }) {
        return Err(CoreError::invalid_input(
            "模型请求缺少用户任务或会话内容（contents 为空），未发送请求；请重新输入任务。",
        ));
    }
    Ok(())
}

fn is_transient(e: &na_common::CoreError) -> bool {
    let msg = e.to_string().to_lowercase();
    msg.contains("error sending request")
        || msg.contains("connection refused")
        || msg.contains("connection reset")
        || msg.contains("timed out")
        || msg.contains("dns error")
        || msg.contains("failed to connect")
}

/// Whether a native tool request was rejected because the endpoint/model does
/// not implement function calling. In Auto mode we can safely retry the same
/// turn through the text ReAct protocol, preserving the tool capability rather
/// than failing the whole writing run.
fn is_tool_compatibility_error(e: &na_common::CoreError) -> bool {
    let lower = e.to_string().to_ascii_lowercase();
    let status = [
        " 400",
        " 404",
        " 405",
        " 422",
        "bad request",
        "unprocessable",
    ]
    .iter()
    .any(|needle| lower.contains(needle));
    let tool_language = [
        "tool",
        "function",
        "schema",
        "unsupported",
        "not support",
        "does not support",
        "not enabled",
        "incompatible",
        "unknown field",
    ]
    .iter()
    .any(|needle| lower.contains(needle));
    status && tool_language
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
                    tokio::time::sleep(std::time::Duration::from_millis(500 * (attempt as u64)))
                        .await;
                }
                let result = self.complete_once(request.clone()).await;
                match result {
                    Ok(resp) => return Ok(resp),
                    Err(error)
                        if self.config.tool_mode == ProviderToolMode::Auto
                            && request.protocol == Protocol::NativeToolCall
                            && !request.tools.is_empty()
                            && is_tool_compatibility_error(&error) =>
                    {
                        let mut fallback = request.clone();
                        fallback.protocol = Protocol::ReActText;
                        return self
                            .complete_once(fallback)
                            .await
                            .map(adapt_react_fallback)
                            .map_err(|fallback_error| {
                                fallback_error.with_context(format!(
                                    "原生工具调用被供应商拒绝，文本兼容模式也失败: {error}"
                                ))
                            });
                    }
                    Err(e) if is_transient(&e) && attempt < 2 => {
                        last_err = Some(e);
                    }
                    Err(e) => return Err(e),
                }
            }
            Err(last_err.unwrap_or_else(|| {
                CoreError::internal("provider retry loop completed without an attempt")
            }))
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
                    tokio::time::sleep(std::time::Duration::from_millis(500 * (attempt as u64)))
                        .await;
                }
                let result = self
                    .complete_streaming_once(request.clone(), on_delta)
                    .await;
                match result {
                    Ok(resp) => return Ok(resp),
                    Err(error)
                        if self.config.tool_mode == ProviderToolMode::Auto
                            && request.protocol == Protocol::NativeToolCall
                            && !request.tools.is_empty()
                            && is_tool_compatibility_error(&error) =>
                    {
                        let mut fallback = request.clone();
                        fallback.protocol = Protocol::ReActText;
                        return self
                            .complete_once(fallback)
                            .await
                            .map(|response| {
                                let adapted = adapt_react_fallback(response);
                                if !adapted.text.is_empty() {
                                    on_delta(&adapted.text);
                                }
                                adapted
                            })
                            .map_err(|fallback_error| {
                                fallback_error.with_context(format!(
                                    "原生工具调用被供应商拒绝，文本兼容模式也失败: {error}"
                                ))
                            });
                    }
                    Err(e) if is_transient(&e) && attempt < 2 => {
                        last_err = Some(e);
                    }
                    Err(e) => return Err(e),
                }
            }
            Err(last_err.unwrap_or_else(|| {
                CoreError::internal("provider retry loop completed without an attempt")
            }))
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
            serde_json::from_str(&s)
                .map_err(|e| CoreError::from(e).with_context("parsing providers.json"))?
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
        self.persist_settings(&self.settings)
    }

    fn persist_settings(&self, settings: &ProviderSettings) -> Result<()> {
        let json = serde_json::to_string_pretty(settings)?;
        atomic_write(&self.path, json.as_bytes(), "providers.json")
    }

    /// Add or replace a provider (by id) and persist.
    pub fn upsert(&mut self, cfg: ProviderConfig) -> Result<()> {
        let mut next = self.settings.clone();
        if let Some(existing) = next.providers.iter_mut().find(|p| p.id == cfg.id) {
            *existing = cfg;
        } else {
            next.providers.push(cfg);
        }
        self.persist_settings(&next)?;
        self.settings = next;
        Ok(())
    }

    /// Remove a provider (by id) and persist; clears active if it was selected.
    pub fn remove(&mut self, id: &str) -> Result<()> {
        let mut next = self.settings.clone();
        next.providers.retain(|p| p.id != id);
        if next.active_provider.as_deref() == Some(id) {
            next.active_provider = None;
            next.active_model = None;
        }
        self.persist_settings(&next)?;
        self.settings = next;
        Ok(())
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
        let mut next = self.settings.clone();
        next.active_provider = Some(provider_id.to_string());
        next.active_model = Some(model.to_string());
        self.persist_settings(&next)?;
        self.settings = next;
        Ok(())
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

    /// Build a live provider with a per-request sampling override.
    ///
    /// The persisted provider defaults remain untouched; callers such as the
    /// Agent discussion surface can tune one turn without changing settings.
    pub fn build_active_with_sampling(
        &self,
        sampling: SamplingParams,
    ) -> Result<HttpModelProvider> {
        let (cfg, model) = self.active().ok_or_else(|| {
            CoreError::invalid_input("尚未选择当前模型供应商，请先在“供应商”里配置并选用")
        })?;
        let mut cfg = cfg.clone();
        cfg.sampling = sampling;
        HttpModelProvider::new(cfg, model)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::Message;
    use na_tools::builtin_registry;

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
    fn openai_merges_parallel_tool_calls_without_repeating_thought() {
        let first = ToolCallRequest::with_id(
            ToolCallId::from_existing("call_1"),
            "read_file",
            json!({ "path": "a.md" }),
        );
        let second = ToolCallRequest::with_id(
            ToolCallId::from_existing("call_2"),
            "read_file",
            json!({ "path": "b.md" }),
        );
        let req = CompletionRequest {
            messages: vec![
                Message::assistant_tool_call("同时读取", first),
                Message::assistant_tool_call("同时读取", second),
            ],
            tools: Vec::new(),
            protocol: Protocol::NativeToolCall,
            sampling: SamplingParams::default(),
        };
        let body = build_openai_body("m", 1024, &req);
        let assistant = &body["messages"][0];
        assert_eq!(body["messages"].as_array().unwrap().len(), 1);
        assert_eq!(assistant["tool_calls"].as_array().unwrap().len(), 2);
        assert_eq!(assistant["content"], "同时读取");
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
    fn parse_openai_accepts_decoded_tool_arguments_from_gateways() {
        let v = json!({
            "choices": [{
                "finish_reason": "tool_calls",
                "message": {
                    "content": null,
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": {
                            "name": "read_file",
                            "arguments": { "path": "chapter.md" }
                        }
                    }]
                }
            }]
        });
        let response = parse_openai_response(&v).unwrap();
        assert_eq!(response.tool_calls[0].args["path"], "chapter.md");
    }

    #[test]
    fn parse_openai_accepts_legacy_function_call_shape() {
        let v = json!({
            "choices": [{
                "finish_reason": "function_call",
                "message": {
                    "content": null,
                    "function_call": {
                        "name": "read_file",
                        "arguments": "{\"path\":\"chapter.md\"}"
                    }
                }
            }]
        });
        let response = parse_openai_response(&v).unwrap();
        assert_eq!(response.finish, FinishReason::ToolUse);
        assert_eq!(response.tool_calls[0].name, "read_file");
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
    fn anthropic_tool_errors_are_marked_as_error_results() {
        let call = ToolCallRequest::with_id(
            ToolCallId::from_existing("tu_error"),
            "read_file",
            json!({ "path": "missing.md" }),
        );
        let result = crate::message::ToolResultRef::new(call.id.clone(), "read_file", false, false);
        let req = CompletionRequest::new(
            vec![
                Message::user("读取文件"),
                Message::assistant_tool_call("我来读取", call),
                Message::tool("文件不存在", result),
            ],
            Vec::new(),
            Protocol::NativeToolCall,
        );
        let body = build_anthropic_body("claude", 1024, &req);
        assert_eq!(body["messages"][2]["content"][0]["type"], "tool_result");
        assert_eq!(body["messages"][2]["content"][0]["is_error"], true);
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
    fn gemini_merges_parallel_function_responses_into_one_user_turn() {
        let first = ToolCallRequest::with_id(
            ToolCallId::from_existing("call_1"),
            "read_file",
            json!({ "path": "a.md" }),
        );
        let second = ToolCallRequest::with_id(
            ToolCallId::from_existing("call_2"),
            "read_file",
            json!({ "path": "b.md" }),
        );
        let req = CompletionRequest {
            messages: vec![
                Message::user("读取两份文件"),
                Message::assistant_tool_call("同时读取", first.clone()),
                Message::assistant_tool_call("同时读取", second.clone()),
                Message::tool(
                    "A",
                    crate::message::ToolResultRef::new(first.id.clone(), "read_file", true, false),
                ),
                Message::tool(
                    "B",
                    crate::message::ToolResultRef::new(second.id.clone(), "read_file", true, false),
                ),
            ],
            tools: Vec::new(),
            protocol: Protocol::NativeToolCall,
            sampling: SamplingParams::default(),
        };
        let body = build_gemini_body(1024, &req);
        let contents = body["contents"].as_array().unwrap();
        assert_eq!(contents.len(), 3);
        assert_eq!(contents[0]["role"], "user");
        assert_eq!(contents[1]["role"], "model");
        assert_eq!(contents[1]["parts"].as_array().unwrap().len(), 3);
        assert_eq!(contents[2]["role"], "user");
        assert_eq!(contents[2]["parts"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn gemini_schema_removes_json_schema_keywords_not_supported_by_gemini() {
        let schema = json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "path": {
                    "type": "string",
                    "minLength": 1,
                    "description": "workspace path"
                },
                "options": {
                    "type": "array",
                    "items": { "type": "string", "additionalProperties": false }
                }
            },
            "required": ["path"]
        });
        let mapped = gemini_schema(&schema);
        // Gemini's REST API uses uppercase enum names even though local tool
        // schemas follow JSON Schema's lowercase names.
        assert_eq!(mapped["type"], "OBJECT");
        assert!(mapped.get("additionalProperties").is_none());
        assert_eq!(mapped["properties"]["path"]["type"], "STRING");
        assert!(mapped["properties"]["path"].get("minLength").is_none());
        assert_eq!(mapped["properties"]["options"]["type"], "ARRAY");
        assert_eq!(mapped["properties"]["options"]["items"]["type"], "STRING");
        assert!(mapped["properties"]["options"]["items"]
            .get("additionalProperties")
            .is_none());
    }

    #[test]
    fn gemini_schema_maps_nullable_unions_to_a_valid_scalar_type() {
        let schema = json!({
            "type": ["null", "string"],
            "description": "optional label"
        });
        let mapped = gemini_schema(&schema);
        assert_eq!(mapped["type"], "STRING");
        assert_eq!(mapped["nullable"], true);
    }

    #[test]
    fn gemini_schema_falls_back_to_uppercase_object_for_invalid_input() {
        assert_eq!(gemini_schema(&json!(true))["type"], "OBJECT");
        assert_eq!(gemini_schema(&json!({}))["type"], "OBJECT");
    }

    #[test]
    fn every_builtin_tool_schema_maps_to_gemini_schema_types() {
        fn assert_types(schema: &Json, tool: &str, path: &str) {
            let object = schema
                .as_object()
                .unwrap_or_else(|| panic!("{tool} schema at {path} is not an object"));
            let type_name = object
                .get("type")
                .and_then(Json::as_str)
                .unwrap_or_else(|| panic!("{tool} schema at {path} has no type"));
            assert!(
                matches!(
                    type_name,
                    "OBJECT" | "STRING" | "NUMBER" | "INTEGER" | "BOOLEAN" | "ARRAY"
                ),
                "{tool} schema at {path} has invalid Gemini type {type_name}"
            );
            if let Some(properties) = object.get("properties").and_then(Json::as_object) {
                for (name, property) in properties {
                    assert_types(property, tool, &format!("{path}.properties.{name}"));
                }
            }
            if let Some(items) = object.get("items") {
                assert_types(items, tool, &format!("{path}.items"));
            }
        }

        for spec in builtin_registry().list_specs() {
            let mapped = gemini_schema(&spec.input_schema);
            assert_types(&mapped, &spec.name, "$");
        }
    }

    #[test]
    fn compatibility_errors_are_detected_for_native_tool_requests() {
        let error = CoreError::model("供应商 P 返回 400: function calling is not supported");
        assert!(is_tool_compatibility_error(&error));
        let unrelated = CoreError::model("供应商 P 返回 500: temporary overload");
        assert!(!is_tool_compatibility_error(&unrelated));
    }

    #[test]
    fn react_fallback_is_adapted_back_to_a_native_tool_call() {
        let response = CompletionResponse::react(
            "Thought: 先读章节\nAction: read_file\nAction Input: {\"path\":\"chapter.md\"}",
        );
        let adapted = adapt_react_fallback(response);
        assert_eq!(adapted.finish, FinishReason::ToolUse);
        assert_eq!(adapted.tool_calls.len(), 1);
        assert_eq!(adapted.tool_calls[0].name, "read_file");
        assert_eq!(adapted.tool_calls[0].args["path"], "chapter.md");
    }

    #[test]
    fn gemini_body_omits_tools_for_react() {
        let body = build_gemini_body(1024, &sample_req(Protocol::ReActText));
        assert!(body.get("tools").is_none());
        assert!(body.get("toolConfig").is_none());
    }

    #[test]
    fn gemini_cache_body_and_application_keep_only_dynamic_contents() {
        let req = CompletionRequest {
            messages: vec![
                Message::system("stable system instructions"),
                Message::user("initial context"),
                Message::assistant("dynamic turn"),
            ],
            tools: Vec::new(),
            protocol: Protocol::NativeToolCall,
            sampling: SamplingParams::default(),
        };
        let (system, contents) = gemini_contents(&req.messages, req.protocol);
        let plan = GeminiCachePlan {
            key: GeminiCacheKey {
                base_url: "https://example.test".into(),
                model: "models/gemini-2.5-flash".into(),
                api_key_hash: 1,
                system_hash: 2,
                prefix_hash: 3,
                prefix_len: 1,
            },
            model_resource: "models/gemini-2.5-flash".into(),
            system: system.unwrap(),
            prefix_contents: vec![contents[0].clone()],
            prefix_len: 1,
        };
        let cache_body = build_gemini_cache_body(&plan);
        assert_eq!(cache_body["model"], "models/gemini-2.5-flash");
        assert!(cache_body["contents"].is_array());

        let mut body = build_gemini_body(256, &req);
        apply_gemini_cache(
            &mut body,
            &GeminiCacheHandle {
                key: plan.key,
                resource_name: "cachedContents/test".into(),
                prefix_len: 1,
            },
        );
        assert_eq!(body["cachedContent"], "cachedContents/test");
        assert!(body.get("systemInstruction").is_none());
        assert_eq!(body["contents"].as_array().unwrap().len(), 1);
        assert_eq!(body["contents"][0]["role"], "model");
    }

    #[test]
    fn gemini_cache_is_disabled_after_a_tool_round_trip_begins() {
        let config = ProviderConfig {
            id: "gemini".into(),
            name: "Gemini".into(),
            protocol: ProviderProtocol::Gemini,
            tool_mode: ProviderToolMode::Auto,
            base_url: "https://generativelanguage.googleapis.com".into(),
            api_key: "key".into(),
            models: vec!["gemini-2.5-flash".into()],
            default_model: None,
            max_tokens: None,
            sampling: SamplingParams::default(),
        };
        let provider = HttpModelProvider::new(config, "gemini-2.5-flash").unwrap();
        let call = ToolCallRequest::new("read_file", json!({ "path": "chapter.md" }));
        let result = crate::message::ToolResultRef::new(call.id.clone(), "read_file", true, false);
        let req = CompletionRequest {
            messages: vec![
                Message::system("stable instructions"),
                Message::assistant_tool_call("read", call),
                Message::tool("chapter", result),
            ],
            tools: vec![],
            protocol: Protocol::NativeToolCall,
            sampling: SamplingParams::default(),
        };
        assert!(provider.gemini_cache_plan(&req).is_none());
    }

    #[test]
    fn parse_gemini_usage_reads_cached_content_tokens() {
        let response = parse_gemini_response(&json!({
            "candidates": [{
                "finishReason": "STOP",
                "content": { "parts": [{ "text": "ok" }] }
            }],
            "usageMetadata": {
                "promptTokenCount": 9000,
                "candidatesTokenCount": 40,
                "totalTokenCount": 9040,
                "cachedContentTokenCount": 8800
            }
        }))
        .unwrap();
        let usage = response.usage.unwrap();
        assert_eq!(usage.prompt_token_count, Some(9000));
        assert_eq!(usage.response_token_count, Some(40));
        assert_eq!(usage.cached_content_token_count, Some(8800));
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
        let resp = finish_accum(text, tools, finish).unwrap();
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
        let resp = finish_accum(text, tools, finish).unwrap();
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
        let resp = finish_accum(text, tools, finish).unwrap();
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
    fn malformed_sse_and_tool_arguments_are_errors() {
        assert!(parse_sse_json("{ malformed").is_err());

        let tools = vec![ToolAccum {
            id: "call_bad".into(),
            name: "write_file".into(),
            args: "{ malformed".into(),
        }];
        assert!(finish_accum(String::new(), tools, Some(FinishReason::ToolUse)).is_err());
    }

    #[test]
    fn trailing_sse_line_is_preserved_without_a_newline() {
        let mut buffer = b"data: {\"ok\":true}".to_vec();
        let line = take_trailing_line(&mut buffer).unwrap();
        assert!(buffer.is_empty());
        let data = sse_data(&line).unwrap();
        assert_eq!(parse_sse_json(data).unwrap()["ok"], true);
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
    fn corrupt_store_is_reported_instead_of_reset() {
        let path = temp_path("corrupt");
        std::fs::write(&path, b"{ not valid json").unwrap();

        let error = ProviderStore::open(&path).unwrap_err();
        assert!(error.is(na_common::ErrorKind::Serialization));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ not valid json");
    }

    #[test]
    fn failed_provider_writes_do_not_change_live_settings() {
        let path = temp_path("write-failure");
        let mut store = ProviderStore::open(&path).unwrap();
        store.upsert(cfg("kept")).unwrap();
        let before = store.settings().clone();
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();

        assert!(store.upsert(cfg("new")).is_err());
        assert_eq!(store.settings(), &before);
        assert!(store.remove("kept").is_err());
        assert_eq!(store.settings(), &before);
        assert!(store.set_active("kept", "m1").is_err());
        assert_eq!(store.settings(), &before);

        let _ = std::fs::remove_dir_all(path);
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
            StatusCode::BAD_REQUEST,
            "function calling is not supported",
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
        let missing = provider_status_message(
            "Q4-Gemini",
            StatusCode::BAD_REQUEST,
            "contents is required",
            &sample_req(Protocol::NativeToolCall),
        );
        assert!(!missing.contains("文本工具"));
        assert!(!is_tool_compatibility_error(&CoreError::model(missing)));
    }

    #[test]
    fn generation_requires_real_dialogue() {
        let mut request = sample_req(Protocol::NativeToolCall);
        request.messages = vec![Message::system("system only")];
        assert!(validate_generation_messages(&request).is_err());
        request.messages.push(Message::user("research this novel"));
        assert!(validate_generation_messages(&request).is_ok());
    }

    #[tokio::test]
    async fn gemini_http_keeps_contents_when_latest_task_exceeds_window() {
        use std::io::{BufRead, BufReader, Read, Write};
        for protocol in [Protocol::NativeToolCall, Protocol::ReActText] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(10)))
                    .unwrap();
                let mut reader = BufReader::new(&mut stream);
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                assert!(line.starts_with("POST /v1beta/models/m1:generateContent "));
                let mut length = 0;
                loop {
                    line.clear();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    if let Some((name, value)) = line.split_once(':') {
                        if name.eq_ignore_ascii_case("content-length") {
                            length = value.trim().parse::<usize>().unwrap();
                        }
                    }
                }
                let mut bytes = vec![0; length];
                reader.read_exact(&mut bytes).unwrap();
                let body: Json = serde_json::from_slice(&bytes).unwrap();
                assert!(!body["contents"].as_array().unwrap().is_empty());
                assert!(body["contents"][0]["parts"][0]["text"]
                    .as_str()
                    .unwrap()
                    .starts_with("研究作品"));
                let response = json!({"candidates": [{"content": {"role":"model", "parts":[{"text":"ok"}]}, "finishReason":"STOP"}]}).to_string();
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).unwrap();
            });
            let mut config = cfg("local-gemini");
            config.protocol = ProviderProtocol::Gemini;
            config.base_url = format!("http://{address}");
            let provider = HttpModelProvider::new(config, "m1").unwrap();
            let messages = crate::ContextManager::default().window(&[Message::user(format!(
                "研究作品{}",
                "长篇作品资料".repeat(2000)
            ))]);
            let response = provider
                .complete(CompletionRequest::new(messages, Vec::new(), protocol))
                .await
                .unwrap();
            assert_eq!(response.text, "ok");
            server.join().unwrap();
        }
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
