//! A minimal, line-delimited JSON-RPC 2.0 layer over the [`Engine`].
//!
//! Each request is one JSON object on its own line; each response is one JSON
//! object on its own line. This is what the GUI shells (Electron / Flutter)
//! speak to the Rust core over stdio (see the `host` binary).
//!
//! Supported methods:
//! * `ping` → `"pong"`.
//! * `list_tools` → array of [`ToolSpec`](na_tools::ToolSpec).
//! * `invoke_tool` `{ name, args }` → [`ToolResult`](na_tools::ToolResult).
//! * `run_goal` `{ goal, title?, protocol?, responses:[CompletionResponse] }`
//!   → `{ outcome, session }` (scripted offline run until a live model is wired).
//! * `cancel` → cancels in-flight work.
//! * Any built-in tool name (`write_file`, `read_file`, `memory_save`,
//!   `memory_recall`, `checkpoint_create`, …) → the params object is used
//!   directly as the tool arguments.

use na_common::{json, CoreError, ErrorKind, Json, Result};
use na_runtime::{CompletionResponse, Protocol};
use serde::{Deserialize, Serialize};

use crate::engine::{outcome_to_json, Engine};

/// An incoming JSON-RPC request.
#[derive(Debug, Clone, Deserialize)]
pub struct RpcRequest {
    /// Protocol marker (ignored; accepted for compatibility).
    #[serde(default)]
    pub jsonrpc: String,
    /// Correlation id echoed back on the response (any JSON; null for notifications).
    #[serde(default)]
    pub id: Json,
    /// Method name.
    pub method: String,
    /// Method parameters (object). Defaults to `null`.
    #[serde(default)]
    pub params: Json,
}

/// A JSON-RPC error object.
#[derive(Debug, Clone, Serialize)]
pub struct RpcErrorObj {
    /// JSON-RPC-ish numeric code.
    pub code: i64,
    /// Human-readable message.
    pub message: String,
    /// Structured error detail (the normalized [`CoreError`], when available).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Json>,
}

/// A JSON-RPC response.
#[derive(Debug, Clone, Serialize)]
pub struct RpcResponse {
    /// Always `"2.0"`.
    pub jsonrpc: &'static str,
    /// Echoed request id.
    pub id: Json,
    /// Success payload.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Json>,
    /// Failure payload.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcErrorObj>,
}

impl RpcResponse {
    fn ok(id: Json, result: Json) -> Self {
        RpcResponse {
            jsonrpc: "2.0",
            id,
            result: Some(result),
            error: None,
        }
    }

    fn err(id: Json, code: i64, message: String, data: Option<Json>) -> Self {
        RpcResponse {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(RpcErrorObj {
                code,
                message,
                data,
            }),
        }
    }
}

/// Map a [`CoreError`] kind to a JSON-RPC-ish error code.
fn error_code(kind: ErrorKind) -> i64 {
    match kind {
        ErrorKind::InvalidInput => -32602, // Invalid params
        ErrorKind::NotFound => -32601,     // Method not found / resource missing
        ErrorKind::PermissionDenied | ErrorKind::SandboxViolation | ErrorKind::SecurityBlocked => {
            -32003
        }
        ErrorKind::Cancelled => -32004,
        _ => -32000, // Generic server error
    }
}

/// The set of built-in tool method names that take their params object directly
/// as the tool arguments.
fn is_direct_tool(method: &str) -> bool {
    matches!(
        method,
        "read_file"
            | "write_file"
            | "list_dir"
            | "edit_file"
            | "search"
            | "shell"
            | "web_fetch"
            | "vcs_commit"
            | "vcs_log"
            | "vcs_diff"
            | "vcs_restore"
            | "vcs_branch"
            | "memory_save"
            | "memory_recall"
            | "memory_list"
            | "memory_classify"
            | "memory_archive"
            | "checkpoint_create"
            | "checkpoint_list"
            | "checkpoint_restore"
            | "skill_list"
            | "skill_load"
            | "spawn_subagent"
    )
}

/// Dispatch one parsed request to the engine, returning the JSON result or a
/// normalized error.
pub async fn dispatch(engine: &Engine, method: &str, params: Json) -> Result<Json> {
    match method {
        "ping" => Ok(json!("pong")),

        "list_tools" => Ok(serde_json::to_value(engine.list_tools())?),

        "invoke_tool" => {
            let name = params
                .get("name")
                .and_then(Json::as_str)
                .ok_or_else(|| CoreError::invalid_input("invoke_tool requires string \"name\""))?;
            let args = params
                .get("args")
                .cloned()
                .unwrap_or_else(|| Json::Object(Default::default()));
            let result = engine.invoke_tool(name, args).await;
            Ok(serde_json::to_value(result)?)
        }

        "run_goal" => {
            let goal = params.get("goal").and_then(Json::as_str).unwrap_or("");
            let title = params
                .get("title")
                .and_then(Json::as_str)
                .unwrap_or("untitled");
            let protocol = match params.get("protocol").and_then(Json::as_str) {
                Some("re_act_text") | Some("react") | Some("react_text") => Protocol::ReActText,
                _ => Protocol::NativeToolCall,
            };
            let responses: Vec<CompletionResponse> = match params.get("responses") {
                Some(v) => serde_json::from_value(v.clone())
                    .map_err(|e| CoreError::invalid_input(format!("invalid \"responses\": {e}")))?,
                None => {
                    return Err(CoreError::invalid_input(
                        "run_goal requires \"responses\" (scripted model output) until a live \
                         model provider is configured",
                    ))
                }
            };
            let (outcome, session) = engine
                .run_goal_scripted(goal, title, protocol, responses)
                .await?;
            Ok(json!({
                "outcome": outcome_to_json(&outcome),
                "session": serde_json::to_value(&session)?,
            }))
        }

        "cancel" => {
            engine.cancel();
            Ok(json!("cancelled"))
        }

        m if is_direct_tool(m) => {
            // The params object is the tool's arguments verbatim.
            let args = if params.is_null() {
                Json::Object(Default::default())
            } else {
                params
            };
            let result = engine.invoke_tool(m, args).await;
            Ok(serde_json::to_value(result)?)
        }

        other => Err(CoreError::not_found(format!("unknown method {other:?}"))),
    }
}

/// Handle one raw request line, returning the serialized response line.
///
/// Returns `None` for a blank line (nothing to answer). A parse failure yields a
/// well-formed JSON-RPC parse-error response rather than crashing the loop.
pub async fn handle_line(engine: &Engine, line: &str) -> Option<String> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }

    let req: RpcRequest = match serde_json::from_str(trimmed) {
        Ok(r) => r,
        Err(e) => {
            let resp = RpcResponse::err(Json::Null, -32700, format!("parse error: {e}"), None);
            return Some(serde_json::to_string(&resp).unwrap_or_default());
        }
    };

    let id = req.id.clone();
    let resp = match dispatch(engine, &req.method, req.params).await {
        Ok(result) => RpcResponse::ok(id, result),
        Err(e) => RpcResponse::err(
            id,
            error_code(e.kind),
            e.message.clone(),
            serde_json::to_value(&e).ok(),
        ),
    };
    Some(serde_json::to_string(&resp).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use na_runtime::ToolCallRequest;

    fn temp_root(tag: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("na_host_rpc_{}_{}", tag, na_common::next_id("t")));
        p
    }

    fn engine(tag: &str) -> Engine {
        Engine::new(temp_root(tag)).unwrap()
    }

    #[tokio::test]
    async fn ping_works() {
        let e = engine("ping");
        let out = dispatch(&e, "ping", Json::Null).await.unwrap();
        assert_eq!(out, json!("pong"));
    }

    #[tokio::test]
    async fn list_tools_returns_all() {
        let e = engine("list");
        let out = dispatch(&e, "list_tools", Json::Null).await.unwrap();
        assert_eq!(out.as_array().unwrap().len(), 23);
    }

    #[tokio::test]
    async fn invoke_tool_write_then_read() {
        let e = engine("inv");
        let w = dispatch(
            &e,
            "invoke_tool",
            json!({ "name": "write_file", "args": { "path": "a.md", "content": "你好" } }),
        )
        .await
        .unwrap();
        assert_eq!(w["ok"], true);

        // direct-tool form (params == args)
        let r = dispatch(&e, "read_file", json!({ "path": "a.md" }))
            .await
            .unwrap();
        assert_eq!(r["ok"], true);
        assert!(r["content"].as_str().unwrap().contains("你好"));
    }

    #[tokio::test]
    async fn run_goal_scripted_via_rpc() {
        let e = engine("goal");
        let call = ToolCallRequest::new("write_file", json!({ "path": "ch1.md", "content": "起" }));
        let responses = json!([
            CompletionResponse::tool_call(call),
            CompletionResponse::answer("完成"),
        ]);
        let out = dispatch(
            &e,
            "run_goal",
            json!({ "goal": "写第一章", "title": "书", "responses": responses }),
        )
        .await
        .unwrap();
        assert_eq!(out["outcome"]["success"], true);
        assert_eq!(out["outcome"]["final_answer"], "完成");
        assert!(out["session"]["messages"].as_array().unwrap().len() >= 2);
    }

    #[tokio::test]
    async fn run_goal_without_responses_is_invalid() {
        let e = engine("goalbad");
        let err = dispatch(&e, "run_goal", json!({ "goal": "x" }))
            .await
            .unwrap_err();
        assert!(err.is(ErrorKind::InvalidInput));
    }

    #[tokio::test]
    async fn unknown_method_is_not_found() {
        let e = engine("unk");
        let err = dispatch(&e, "frobnicate", Json::Null).await.unwrap_err();
        assert!(err.is(ErrorKind::NotFound));
    }

    #[tokio::test]
    async fn handle_line_parse_error_is_wellformed() {
        let e = engine("parse");
        let resp = handle_line(&e, "{ not json }").await.unwrap();
        let v: Json = serde_json::from_str(&resp).unwrap();
        assert_eq!(v["jsonrpc"], "2.0");
        assert_eq!(v["error"]["code"], -32700);
    }

    #[tokio::test]
    async fn handle_line_echoes_id_and_result() {
        let e = engine("echo");
        let resp = handle_line(&e, r#"{"jsonrpc":"2.0","id":7,"method":"ping"}"#)
            .await
            .unwrap();
        let v: Json = serde_json::from_str(&resp).unwrap();
        assert_eq!(v["id"], 7);
        assert_eq!(v["result"], "pong");
    }

    #[tokio::test]
    async fn handle_line_blank_is_none() {
        let e = engine("blank");
        assert!(handle_line(&e, "   ").await.is_none());
    }

    #[tokio::test]
    async fn error_carries_normalized_data() {
        let e = engine("errdata");
        // read a missing file -> tool returns an error ToolResult (ok=false), but
        // the RPC call itself succeeds (the tool lifecycle never throws). Assert
        // the structured error surfaces inside the result.
        let r = dispatch(&e, "read_file", json!({ "path": "missing.md" }))
            .await
            .unwrap();
        assert_eq!(r["ok"], false);
        assert!(r["content"].as_str().unwrap().starts_with("[error:"));
    }
}
