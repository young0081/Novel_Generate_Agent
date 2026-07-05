//! A real, line-delimited JSON-RPC [`McpClient`] over a child process's stdio.
//!
//! The Model-Context-Protocol commonly runs a server as a subprocess that speaks
//! JSON-RPC 2.0 over stdin/stdout, one JSON object per line. This module provides
//! that client without any external crates, plus a transport seam so the whole
//! thing is testable **offline**:
//!
//! * [`McpTransport`] — the seam: one `request(payload) -> response` round-trip
//!   of a single JSON line.
//! * [`StdioTransport`] — spawns a child process (`program` + `args`) via
//!   [`tokio::process`], writes one request line to its stdin and reads one
//!   response line from its stdout, under a timeout. (Used in production; **not**
//!   exercised against a real server in tests.)
//! * [`InMemoryTransport`] — an in-process transport backed by a closure
//!   `Fn(String) -> String`, used to simulate an MCP server in tests.
//! * [`StdioMcpClient`] — wraps any [`McpTransport`] and implements
//!   [`McpClient`]: `tools/list` parses the advertised tools into
//!   [`ToolSpec`]s; `tools/call` returns the result JSON. JSON-RPC error
//!   responses are mapped to [`CoreError`].
//!
//! Each request carries a monotonically increasing `id`; the response `id` is
//! checked to match.

use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use na_common::{json, CoreError, Json, Result};
use na_sandbox::Capability;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::Mutex;

use crate::tool::{BoxFuture, McpClient, ToolSpec};

/// A request/response transport for a single line-delimited JSON-RPC message.
///
/// Object-safe and `Send + Sync` so a [`StdioMcpClient`] can hold one behind a
/// generic or a trait object and be shared across tasks.
pub trait McpTransport: Send + Sync {
    /// Send one JSON `payload` line and return the single JSON response line.
    fn request<'a>(&'a self, payload: String) -> BoxFuture<'a, Result<String>>;
}

/// A transport that spawns a child process and talks JSON-RPC over its stdio.
///
/// Each [`request`](McpTransport::request) spawns a fresh child, writes the
/// request line to stdin (then closes it), and reads the first line from stdout.
/// This "one process per request" model keeps the transport simple and robust
/// (no shared mutable child state, no half-consumed pipes) which is adequate for
/// the low call volume of tool discovery / invocation.
#[derive(Debug, Clone)]
pub struct StdioTransport {
    program: String,
    args: Vec<String>,
    timeout_ms: u64,
}

impl StdioTransport {
    /// A transport that runs `program` with `args`, with a default 30s timeout.
    pub fn new(
        program: impl Into<String>,
        args: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        StdioTransport {
            program: program.into(),
            args: args.into_iter().map(Into::into).collect(),
            timeout_ms: 30_000,
        }
    }

    /// Set the per-request timeout in milliseconds (builder style).
    pub fn timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = timeout_ms;
        self
    }

    async fn request_inner(&self, payload: String) -> Result<String> {
        let mut child = Command::new(&self.program)
            .args(&self.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| {
                CoreError::from(e).with_context(format!("spawning MCP server {:?}", self.program))
            })?;

        // Write the request line and close stdin so the child can finish reading.
        {
            let mut stdin = child
                .stdin
                .take()
                .ok_or_else(|| CoreError::internal("MCP child has no stdin"))?;
            stdin
                .write_all(payload.as_bytes())
                .await
                .map_err(|e| CoreError::from(e).with_context("writing MCP request"))?;
            stdin
                .write_all(b"\n")
                .await
                .map_err(|e| CoreError::from(e).with_context("writing MCP newline"))?;
            stdin
                .flush()
                .await
                .map_err(|e| CoreError::from(e).with_context("flushing MCP request"))?;
            // `stdin` drops here -> EOF for the child.
        }

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| CoreError::internal("MCP child has no stdout"))?;
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        let n = reader
            .read_line(&mut line)
            .await
            .map_err(|e| CoreError::from(e).with_context("reading MCP response line"))?;
        if n == 0 {
            return Err(CoreError::protocol(
                "MCP server closed stdout without a response",
            ));
        }
        // Best-effort reap; the child will be killed on drop regardless.
        let _ = child.start_kill();
        Ok(line)
    }
}

impl McpTransport for StdioTransport {
    fn request<'a>(&'a self, payload: String) -> BoxFuture<'a, Result<String>> {
        Box::pin(async move {
            let timeout = Duration::from_millis(self.timeout_ms.max(1));
            match tokio::time::timeout(timeout, self.request_inner(payload)).await {
                Ok(inner) => inner,
                Err(_elapsed) => Err(CoreError::timeout(format!(
                    "MCP request to {:?} exceeded {} ms",
                    self.program, self.timeout_ms
                ))),
            }
        })
    }
}

/// An in-process transport for tests: a closure maps a request line to a
/// response line, simulating an MCP server with no IO.
#[derive(Clone)]
pub struct InMemoryTransport {
    handler: Arc<dyn Fn(String) -> String + Send + Sync>,
}

impl std::fmt::Debug for InMemoryTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InMemoryTransport").finish_non_exhaustive()
    }
}

impl InMemoryTransport {
    /// Build from a request→response closure.
    pub fn new(handler: impl Fn(String) -> String + Send + Sync + 'static) -> Self {
        InMemoryTransport {
            handler: Arc::new(handler),
        }
    }
}

impl McpTransport for InMemoryTransport {
    fn request<'a>(&'a self, payload: String) -> BoxFuture<'a, Result<String>> {
        let response = (self.handler)(payload);
        Box::pin(async move { Ok(response) })
    }
}

/// A JSON-RPC MCP client over any [`McpTransport`].
#[derive(Debug)]
pub struct StdioMcpClient<T: McpTransport> {
    transport: T,
    next_id: AtomicU64,
    /// Serializes requests so concurrent calls don't interleave on a single
    /// stdio transport (and keeps id↔response pairing strict).
    lock: Mutex<()>,
}

impl StdioMcpClient<StdioTransport> {
    /// Spawn an MCP server process and wrap it (one process per request).
    pub fn spawn(
        program: impl Into<String>,
        args: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        StdioMcpClient::with_transport(StdioTransport::new(program, args))
    }
}

impl<T: McpTransport> StdioMcpClient<T> {
    /// Wrap an arbitrary transport.
    pub fn with_transport(transport: T) -> Self {
        StdioMcpClient {
            transport,
            next_id: AtomicU64::new(1),
            lock: Mutex::new(()),
        }
    }

    /// Borrow the underlying transport.
    pub fn transport(&self) -> &T {
        &self.transport
    }

    /// Perform one JSON-RPC call: build the request, send it, parse + validate
    /// the response envelope, and return the `result` value.
    async fn rpc(&self, method: &str, params: Json) -> Result<Json> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let mut request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
        });
        if !params.is_null() {
            request["params"] = params;
        }
        let payload = serde_json::to_string(&request).map_err(CoreError::from)?;

        let raw = {
            // Hold the lock across the round-trip so ids and responses stay paired.
            let _guard = self.lock.lock().await;
            self.transport.request(payload).await?
        };

        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(CoreError::protocol(format!(
                "empty MCP response for method {method:?}"
            )));
        }
        let value: Json = serde_json::from_str(trimmed).map_err(|e| {
            CoreError::protocol(format!("invalid JSON-RPC response for {method:?}: {e}"))
        })?;

        // JSON-RPC error object -> map to a CoreError.
        if let Some(error) = value.get("error") {
            let code = error.get("code").and_then(Json::as_i64).unwrap_or(0);
            let message = error
                .get("message")
                .and_then(Json::as_str)
                .unwrap_or("unknown MCP error");
            let full = format!("MCP error {code} for {method:?}: {message}");
            // Server-side / internal JSON-RPC errors map to `model`; malformed or
            // client-shape problems map to `protocol`.
            return Err(if (-32099..=-32000).contains(&code) {
                CoreError::model(full)
            } else {
                CoreError::protocol(full)
            });
        }

        // Validate the response id matches our request id (when present).
        if let Some(resp_id) = value.get("id").and_then(Json::as_u64) {
            if resp_id != id {
                return Err(CoreError::protocol(format!(
                    "MCP response id {resp_id} does not match request id {id}"
                )));
            }
        }

        value.get("result").cloned().ok_or_else(|| {
            CoreError::protocol(format!("MCP response for {method:?} has no result"))
        })
    }
}

/// Parse a single MCP tool description object into a [`ToolSpec`].
///
/// Accepts the standard MCP shape `{ "name", "description", "inputSchema" }` and
/// tolerates `input_schema` as an alias; missing fields get safe defaults.
fn parse_tool_spec(value: &Json) -> Result<ToolSpec> {
    let name = value
        .get("name")
        .and_then(Json::as_str)
        .ok_or_else(|| CoreError::protocol("MCP tool entry missing \"name\""))?
        .to_string();
    let description = value
        .get("description")
        .and_then(Json::as_str)
        .unwrap_or("")
        .to_string();
    let input_schema = value
        .get("inputSchema")
        .or_else(|| value.get("input_schema"))
        .cloned()
        .unwrap_or_else(|| json!({ "type": "object" }));

    Ok(ToolSpec::new(
        name,
        description,
        input_schema,
        // Remote tools reach the network; gate them behind NetworkAccess.
        vec![Capability::NetworkAccess],
        // Conservatively treat remote tools as potentially mutating.
        true,
    ))
}

impl<T: McpTransport> McpClient for StdioMcpClient<T> {
    fn list_tools<'a>(&'a self) -> BoxFuture<'a, Result<Vec<ToolSpec>>> {
        Box::pin(async move {
            let result = self.rpc("tools/list", Json::Null).await?;
            // MCP returns { "tools": [ ... ] }; tolerate a bare array too.
            let tools_array = result
                .get("tools")
                .and_then(Json::as_array)
                .or_else(|| result.as_array())
                .ok_or_else(|| {
                    CoreError::protocol("MCP tools/list result has no \"tools\" array")
                })?;
            let mut specs = Vec::with_capacity(tools_array.len());
            for entry in tools_array {
                specs.push(parse_tool_spec(entry)?);
            }
            Ok(specs)
        })
    }

    fn call_tool<'a>(&'a self, name: &'a str, args: Json) -> BoxFuture<'a, Result<Json>> {
        Box::pin(async move {
            let params = json!({ "name": name, "arguments": args });
            self.rpc("tools/call", params).await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build an InMemory-backed client whose server echoes a scripted protocol:
    /// it inspects the request's `method` and `id` and returns a matching reply.
    fn scripted_client() -> StdioMcpClient<InMemoryTransport> {
        let transport = InMemoryTransport::new(|req: String| {
            let value: Json = serde_json::from_str(req.trim()).unwrap();
            let id = value.get("id").cloned().unwrap_or(Json::Null);
            let method = value.get("method").and_then(Json::as_str).unwrap_or("");
            match method {
                "tools/list" => json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "tools": [
                            {
                                "name": "lore_lookup",
                                "description": "Look up world lore",
                                "inputSchema": {
                                    "type": "object",
                                    "required": ["topic"],
                                    "properties": { "topic": { "type": "string" } }
                                }
                            },
                            {
                                "name": "name_generator",
                                "description": "Generate a fantasy name"
                            }
                        ]
                    }
                })
                .to_string(),
                "tools/call" => {
                    let tool = value
                        .get("params")
                        .and_then(|p| p.get("name"))
                        .and_then(Json::as_str)
                        .unwrap_or("");
                    if tool == "lore_lookup" {
                        let topic = value
                            .get("params")
                            .and_then(|p| p.get("arguments"))
                            .and_then(|a| a.get("topic"))
                            .and_then(Json::as_str)
                            .unwrap_or("");
                        json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": { "content": format!("lore about {topic}") }
                        })
                        .to_string()
                    } else {
                        // Unknown tool -> JSON-RPC error.
                        json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "error": { "code": -32601, "message": "method not found" }
                        })
                        .to_string()
                    }
                }
                _ => json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32601, "message": "method not found" }
                })
                .to_string(),
            }
        });
        StdioMcpClient::with_transport(transport)
    }

    #[tokio::test]
    async fn list_tools_parses_specs() {
        let client = scripted_client();
        let tools = client.list_tools().await.unwrap();
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].name, "lore_lookup");
        assert_eq!(tools[0].description, "Look up world lore");
        assert!(tools[0].input_schema.is_object());
        assert_eq!(tools[0].capabilities, vec![Capability::NetworkAccess]);
        assert!(tools[0].mutating);
        // Second tool had no inputSchema -> defaulted to an object schema.
        assert_eq!(tools[1].name, "name_generator");
        assert_eq!(tools[1].input_schema, json!({ "type": "object" }));
    }

    #[tokio::test]
    async fn call_tool_returns_result() {
        let client = scripted_client();
        let result = client
            .call_tool("lore_lookup", json!({ "topic": "霜寒剑" }))
            .await
            .unwrap();
        assert_eq!(result["content"], "lore about 霜寒剑");
    }

    #[tokio::test]
    async fn jsonrpc_error_becomes_core_error() {
        let client = scripted_client();
        let err = client
            .call_tool("does_not_exist", json!({}))
            .await
            .unwrap_err();
        // -32601 is outside the -32000..-32099 server-error band -> protocol.
        assert!(err.is(na_common::ErrorKind::Protocol), "{err}");
        assert!(err.message.contains("method not found"));
    }

    #[tokio::test]
    async fn server_error_band_maps_to_model() {
        let transport = InMemoryTransport::new(|req: String| {
            let value: Json = serde_json::from_str(req.trim()).unwrap();
            let id = value.get("id").cloned().unwrap_or(Json::Null);
            json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32050, "message": "server exploded" }
            })
            .to_string()
        });
        let client = StdioMcpClient::with_transport(transport);
        let err = client.call_tool("x", json!({})).await.unwrap_err();
        assert!(err.is(na_common::ErrorKind::Model), "{err}");
    }

    #[tokio::test]
    async fn mismatched_id_is_protocol_error() {
        let transport = InMemoryTransport::new(|_req: String| {
            // Always reply with the wrong id (999).
            json!({ "jsonrpc": "2.0", "id": 999, "result": { "ok": true } }).to_string()
        });
        let client = StdioMcpClient::with_transport(transport);
        let err = client.list_tools().await.unwrap_err();
        assert!(err.is(na_common::ErrorKind::Protocol), "{err}");
        assert!(err.message.contains("does not match"));
    }

    #[tokio::test]
    async fn invalid_json_response_is_protocol_error() {
        let transport = InMemoryTransport::new(|_req: String| "not json at all".to_string());
        let client = StdioMcpClient::with_transport(transport);
        let err = client.list_tools().await.unwrap_err();
        assert!(err.is(na_common::ErrorKind::Protocol), "{err}");
    }

    #[tokio::test]
    async fn missing_result_is_protocol_error() {
        let transport = InMemoryTransport::new(|req: String| {
            let value: Json = serde_json::from_str(req.trim()).unwrap();
            let id = value.get("id").cloned().unwrap_or(Json::Null);
            // A response with neither result nor error.
            json!({ "jsonrpc": "2.0", "id": id }).to_string()
        });
        let client = StdioMcpClient::with_transport(transport);
        let err = client.call_tool("x", json!({})).await.unwrap_err();
        assert!(err.is(na_common::ErrorKind::Protocol), "{err}");
        assert!(err.message.contains("no result"));
    }

    #[tokio::test]
    async fn works_as_mcp_client_trait_object() {
        // Prove the client is object-safe and usable as Arc<dyn McpClient>, and
        // that McpTool::discover can consume it.
        let client: Arc<dyn McpClient> = Arc::new(scripted_client());
        let tools = crate::tools::mcp::McpTool::discover(client.as_ref())
            .await
            .unwrap();
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].remote_name(), "lore_lookup");
    }

    #[tokio::test]
    async fn ids_increment_across_calls() {
        // Capture the ids seen by the server to confirm they advance.
        use std::sync::Mutex as StdMutex;
        let seen = Arc::new(StdMutex::new(Vec::<u64>::new()));
        let seen2 = seen.clone();
        let transport = InMemoryTransport::new(move |req: String| {
            let value: Json = serde_json::from_str(req.trim()).unwrap();
            let id = value.get("id").and_then(Json::as_u64).unwrap();
            seen2.lock().unwrap().push(id);
            json!({ "jsonrpc": "2.0", "id": id, "result": { "tools": [] } }).to_string()
        });
        let client = StdioMcpClient::with_transport(transport);
        client.list_tools().await.unwrap();
        client.list_tools().await.unwrap();
        let ids = seen.lock().unwrap().clone();
        assert_eq!(ids.len(), 2);
        assert!(ids[1] > ids[0], "ids should increase: {ids:?}");
    }

    // ---- StdioTransport against a real (cross-platform) helper process ----
    //
    // This does NOT contact the network or any external MCP server: it runs a
    // tiny in-tree script via the system shell that echoes a canned JSON-RPC
    // line, exercising the actual spawn/stdin/stdout path of StdioTransport.

    #[tokio::test]
    async fn stdio_transport_round_trips_via_shell_echo() {
        // A canned JSON-RPC response line the "server" prints regardless of input.
        // We must match whatever id the client used (1 for the first call).
        let response = r#"{"jsonrpc":"2.0","id":1,"result":{"tools":[]}}"#;

        let transport = if cfg!(windows) {
            // cmd.exe mangles JSON quotes on the command line, so write the canned
            // response to a temp file and `type` it to stdout instead.
            let dir = std::env::temp_dir();
            let path = dir.join(format!("na_mcp_{}.json", na_common::next_id("t")));
            std::fs::write(&path, format!("{response}\n")).unwrap();
            StdioTransport::new("cmd", ["/C", "type", path.to_str().unwrap()]).timeout_ms(5_000)
        } else {
            // POSIX sh: print the JSON line.
            StdioTransport::new("sh", ["-c", &format!("printf '%s\\n' '{response}'")])
                .timeout_ms(5_000)
        };

        let client = StdioMcpClient::with_transport(transport);
        let tools = client.list_tools().await.unwrap();
        assert!(tools.is_empty());
    }

    #[tokio::test]
    async fn stdio_transport_timeout_on_silent_child() {
        // A child that produces no output before the deadline.
        let transport = if cfg!(windows) {
            // ping introduces a delay without printing to our captured stdout
            // line promptly; redirect its stdout to NUL so read_line blocks.
            StdioTransport::new("cmd", ["/C", "ping -n 6 127.0.0.1 >NUL"]).timeout_ms(150)
        } else {
            StdioTransport::new("sh", ["-c", "sleep 5"]).timeout_ms(150)
        };
        let client = StdioMcpClient::with_transport(transport);
        let err = client.list_tools().await.unwrap_err();
        assert!(err.is(na_common::ErrorKind::Timeout), "{err}");
    }
}
