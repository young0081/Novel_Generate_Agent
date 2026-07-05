//! Conversation messages — the atoms of a [`Session`](crate::session::Session).
//!
//! A creation session is, at heart, an ordered list of [`Message`]s exchanged
//! between the four [`Role`]s. Each message carries a stable [`MessageId`], a
//! creation timestamp, and — for the two tool-related kinds — an optional
//! structured payload:
//!
//! * an *assistant* message may carry a [`ToolCallRequest`] (the model asking to
//!   run a tool), and
//! * a *tool* message may carry a [`ToolResultRef`] (the observation produced by
//!   running that tool).
//!
//! Everything here is `Serialize`/`Deserialize` so a whole session round-trips
//! to and from disk losslessly (see [`Session::save`](crate::session::Session::save)).

use na_common::time::now_millis;
use na_common::{Json, MessageId, ToolCallId};
use serde::{Deserialize, Serialize};

/// Who authored a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// The system preamble (instructions, persona, tool catalog). Never dropped
    /// by context windowing.
    System,
    /// The human user.
    User,
    /// The model / assistant.
    Assistant,
    /// A tool observation fed back into the conversation.
    Tool,
}

impl Role {
    /// Stable lowercase label (matches the serde representation).
    pub fn as_str(self) -> &'static str {
        match self {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "tool",
        }
    }
}

/// A request from the model to invoke a named tool with JSON arguments.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCallRequest {
    /// Correlation id linking this request to its [`ToolResultRef`].
    pub id: ToolCallId,
    /// The tool to call (must exist in the registry).
    pub name: String,
    /// Arguments object passed to the tool (validated against its schema).
    pub args: Json,
}

impl ToolCallRequest {
    /// Build a request, allocating a fresh [`ToolCallId`].
    pub fn new(name: impl Into<String>, args: Json) -> Self {
        ToolCallRequest {
            id: ToolCallId::new(),
            name: name.into(),
            args,
        }
    }

    /// Build a request reusing an explicit call id (e.g. echoing a native
    /// tool-call id supplied by a provider).
    pub fn with_id(id: ToolCallId, name: impl Into<String>, args: Json) -> Self {
        ToolCallRequest {
            id,
            name: name.into(),
            args,
        }
    }
}

/// A compact reference to a tool's result, suitable for embedding in a message
/// and persisting. The heavy structured `data` of a `ToolResult` is *not* kept
/// here — only the model-facing essentials plus provenance flags.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolResultRef {
    /// Correlation id matching the originating [`ToolCallRequest`].
    pub call_id: ToolCallId,
    /// The tool that produced this result.
    pub name: String,
    /// Whether the tool succeeded.
    pub ok: bool,
    /// `true` when the content came from outside the workspace (web/MCP) and was
    /// run through the prompt-injection guard before entering the context.
    pub untrusted: bool,
}

impl ToolResultRef {
    /// Construct a result reference.
    pub fn new(call_id: ToolCallId, name: impl Into<String>, ok: bool, untrusted: bool) -> Self {
        ToolResultRef {
            call_id,
            name: name.into(),
            ok,
            untrusted,
        }
    }
}

/// One message in a conversation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    /// Stable unique id.
    pub id: MessageId,
    /// Who authored it.
    pub role: Role,
    /// The textual body (already processed / sanitized when it is a tool result).
    pub content: String,
    /// Set on an assistant message that requests a tool call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call: Option<ToolCallRequest>,
    /// Set on a tool message that carries an observation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_result: Option<ToolResultRef>,
    /// Creation time (epoch millis).
    pub ts_ms: u64,
}

impl Message {
    /// Build a message with the given role and content, stamping a fresh id and
    /// the current time. Use the [`system`](Self::system) / [`user`](Self::user)
    /// / [`assistant`](Self::assistant) / [`tool`](Self::tool) helpers for the
    /// common cases.
    pub fn new(role: Role, content: impl Into<String>) -> Self {
        Message {
            id: MessageId::new(),
            role,
            content: content.into(),
            tool_call: None,
            tool_result: None,
            ts_ms: now_millis(),
        }
    }

    /// A system message (instructions / persona / tool catalog).
    pub fn system(content: impl Into<String>) -> Self {
        Message::new(Role::System, content)
    }

    /// A user message.
    pub fn user(content: impl Into<String>) -> Self {
        Message::new(Role::User, content)
    }

    /// A plain assistant message (no tool call).
    pub fn assistant(content: impl Into<String>) -> Self {
        Message::new(Role::Assistant, content)
    }

    /// An assistant message that requests a tool call. `content` is the model's
    /// surrounding thought / narration (may be empty).
    pub fn assistant_tool_call(content: impl Into<String>, call: ToolCallRequest) -> Self {
        let mut m = Message::new(Role::Assistant, content);
        m.tool_call = Some(call);
        m
    }

    /// A tool observation message carrying the (already sanitized) `content` and
    /// a [`ToolResultRef`] linking it back to the originating call.
    pub fn tool(content: impl Into<String>, result: ToolResultRef) -> Self {
        let mut m = Message::new(Role::Tool, content);
        m.tool_result = Some(result);
        m
    }

    /// Attach a tool-call request (builder style).
    pub fn with_tool_call(mut self, call: ToolCallRequest) -> Self {
        self.tool_call = Some(call);
        self
    }

    /// Attach a tool-result reference (builder style).
    pub fn with_tool_result(mut self, result: ToolResultRef) -> Self {
        self.tool_result = Some(result);
        self
    }

    /// Whether this is a system message (kept by context windowing).
    pub fn is_system(&self) -> bool {
        self.role == Role::System
    }

    /// Whether this message requests a tool call.
    pub fn has_tool_call(&self) -> bool {
        self.tool_call.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use na_common::json;

    #[test]
    fn role_serializes_snake_case() {
        assert_eq!(serde_json::to_string(&Role::System).unwrap(), "\"system\"");
        assert_eq!(
            serde_json::to_string(&Role::Assistant).unwrap(),
            "\"assistant\""
        );
        let r: Role = serde_json::from_str("\"tool\"").unwrap();
        assert_eq!(r, Role::Tool);
    }

    #[test]
    fn builders_set_role_and_content() {
        assert_eq!(Message::system("s").role, Role::System);
        assert_eq!(Message::user("u").content, "u");
        assert_eq!(Message::assistant("a").role, Role::Assistant);
    }

    #[test]
    fn assistant_tool_call_carries_request() {
        let call = ToolCallRequest::new("write_file", json!({ "path": "a.md" }));
        let m = Message::assistant_tool_call("writing", call.clone());
        assert!(m.has_tool_call());
        assert_eq!(m.tool_call.as_ref().unwrap().name, "write_file");
        assert_eq!(m.tool_call.unwrap().id, call.id);
    }

    #[test]
    fn tool_message_carries_result_ref() {
        let call_id = ToolCallId::new();
        let r = ToolResultRef::new(call_id.clone(), "web_fetch", true, true);
        let m = Message::tool("data", r);
        assert_eq!(m.role, Role::Tool);
        let tr = m.tool_result.unwrap();
        assert_eq!(tr.call_id, call_id);
        assert!(tr.untrusted);
    }

    #[test]
    fn message_round_trips_json_with_tool_call() {
        let call = ToolCallRequest::with_id(
            ToolCallId::from_existing("call_fixed"),
            "search",
            json!({ "content_regex": "龙王" }),
        );
        let m = Message::assistant_tool_call("", call);
        let s = serde_json::to_string(&m).unwrap();
        let back: Message = serde_json::from_str(&s).unwrap();
        assert_eq!(m, back);
    }

    #[test]
    fn plain_message_omits_optional_fields_in_json() {
        let m = Message::user("hello");
        let v = serde_json::to_value(&m).unwrap();
        assert!(v.get("tool_call").is_none());
        assert!(v.get("tool_result").is_none());
        // round trips too
        let back: Message = serde_json::from_value(v).unwrap();
        assert_eq!(m, back);
    }

    #[test]
    fn ids_are_unique_and_timestamped() {
        let a = Message::user("a");
        let b = Message::user("b");
        assert_ne!(a.id, b.id);
        assert!(a.ts_ms > 0);
    }
}
