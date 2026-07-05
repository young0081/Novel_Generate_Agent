//! MCP adapter: expose a remote Model-Context-Protocol tool as a local [`Tool`].
//!
//! The [`McpClient`](crate::McpClient) trait (in [`crate::tool`]) abstracts a
//! remote tool server. [`McpTool`] wraps one named remote tool so the registry
//! can treat it like any other local tool: it forwards the validated arguments
//! to `ctx.mcp.call_tool`, wraps the JSON response, and marks the result
//! `untrusted` (remote output, like web content, must not be trusted as
//! instructions).
//!
//! Use [`McpTool::discover`] to turn every tool a client advertises into a set
//! of `McpTool`s ready to register.

use na_common::{json, CoreError, Json, Result};
use na_sandbox::Capability;

use crate::output::OutputProcessor;
use crate::tool::{BoxFuture, McpClient, ResultMeta, Tool, ToolContext, ToolResult, ToolSpec};

/// A local [`Tool`] that proxies to a single remote MCP tool by name.
#[derive(Debug, Clone)]
pub struct McpTool {
    spec: ToolSpec,
}

impl McpTool {
    /// Wrap a remote tool described by `spec`. The local tool name is the same
    /// as the remote name; capabilities default to [`NetworkAccess`] unless the
    /// remote spec already declares some.
    pub fn new(mut spec: ToolSpec) -> Self {
        if spec.capabilities.is_empty() {
            spec.capabilities = vec![Capability::NetworkAccess];
        }
        McpTool { spec }
    }

    /// Ask `client` for its tool list and wrap each one as an [`McpTool`].
    pub async fn discover(client: &dyn McpClient) -> Result<Vec<McpTool>> {
        let specs = client.list_tools().await?;
        Ok(specs.into_iter().map(McpTool::new).collect())
    }

    /// The remote tool name this adapter proxies.
    pub fn remote_name(&self) -> &str {
        &self.spec.name
    }
}

impl Tool for McpTool {
    fn spec(&self) -> ToolSpec {
        self.spec.clone()
    }

    fn execute<'a>(
        &'a self,
        args: Json,
        ctx: &'a ToolContext,
    ) -> BoxFuture<'a, Result<ToolResult>> {
        Box::pin(async move {
            let name = self.spec.name.clone();
            let value = ctx
                .mcp
                .call_tool(&name, args)
                .await
                .map_err(|e| e.with_context(format!("calling MCP tool {name}")))?;

            // Render the JSON result as text for the model (pretty but bounded),
            // marking it untrusted.
            let pretty = serde_json::to_string_pretty(&value).map_err(CoreError::from)?;
            let processed = OutputProcessor::default().process(pretty.as_bytes());

            let meta = ResultMeta {
                bytes: processed.bytes,
                truncated: processed.truncated,
                was_binary: processed.was_binary,
                redactions: processed.redactions,
                untrusted: true,
                duration_ms: 0,
            };
            Ok(ToolResult {
                ok: true,
                content: processed.text,
                data: json!({ "tool": name, "result": value, "untrusted": true }),
                summary: Some(format!("mcp:{name}")),
                metadata: meta,
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool::{MockMcpClient, ToolContextBuilder};
    use std::sync::Arc;

    fn remote_spec() -> ToolSpec {
        ToolSpec::new(
            "weather",
            "Get the weather",
            json!({
                "type": "object",
                "required": ["city"],
                "properties": { "city": { "type": "string" } }
            }),
            vec![],
            false,
        )
    }

    fn ctx_with_mcp(tag: &str, mcp: Arc<dyn McpClient>) -> ToolContext {
        let mut p = std::env::temp_dir();
        p.push(format!("na_tools_mcp_{}_{}", tag, na_common::next_id("t")));
        ToolContextBuilder::new(p).mcp(mcp).build().unwrap()
    }

    #[tokio::test]
    async fn discover_wraps_remote_tools() {
        let mcp = MockMcpClient::new().with_tool(remote_spec(), json!({ "temp": 20 }));
        let tools = McpTool::discover(&mcp).await.unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].remote_name(), "weather");
        // empty caps -> defaulted to NetworkAccess
        assert_eq!(
            tools[0].spec().capabilities,
            vec![Capability::NetworkAccess]
        );
    }

    #[tokio::test]
    async fn execute_forwards_and_marks_untrusted() {
        let mcp = Arc::new(
            MockMcpClient::new().with_tool(remote_spec(), json!({ "temp": 20, "unit": "C" })),
        );
        let c = ctx_with_mcp("call", mcp);
        let tool = McpTool::new(remote_spec());
        let res = tool
            .execute(json!({ "city": "Shanghai" }), &c)
            .await
            .unwrap();
        assert!(res.ok);
        assert!(res.metadata.untrusted);
        assert_eq!(res.data["result"]["temp"], 20);
        assert!(res.content.contains("\"temp\""));
    }

    #[tokio::test]
    async fn execute_missing_remote_tool_errors() {
        // Client knows nothing about "weather".
        let mcp = Arc::new(MockMcpClient::new());
        let c = ctx_with_mcp("missing", mcp);
        let tool = McpTool::new(remote_spec());
        let err = tool.execute(json!({ "city": "X" }), &c).await.unwrap_err();
        assert!(err.is(na_common::ErrorKind::NotFound));
    }
}
