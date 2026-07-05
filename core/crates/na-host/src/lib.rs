//! `na-host` — the integration layer that fronts the whole Novel Generate Team
//! core for the GUI shells.
//!
//! It exposes two things:
//!
//! * [`Engine`] — an in-process facade bundling a [`ToolRegistry`](na_tools::ToolRegistry)
//!   of every built-in tool with a shared [`ToolContext`](na_tools::ToolContext)
//!   (workspace jail, permission policy, memory / checkpoint / audit stores). The
//!   methods (`invoke_tool`, `run_goal_*`, `cancel`, …) are what the Electron and
//!   Flutter shells ultimately call.
//!
//! * [`rpc`] — a line-delimited JSON-RPC 2.0 protocol over the engine, spoken on
//!   stdio by the `host` binary so a GUI process can drive the core out-of-process.
//!
//! The `demo` binary runs a full, offline, end-to-end agent session to prove the
//! whole stack works together; the `host` binary is the long-running backend.

#![forbid(unsafe_code)]

pub mod engine;
pub mod rpc;

pub use engine::{outcome_to_json, Engine};
pub use rpc::{dispatch, handle_line, RpcErrorObj, RpcRequest, RpcResponse};

// Convenience re-exports so a shell only needs to depend on `na-host`.
pub use na_common::{CoreError, Json, Result};
pub use na_runtime::message::ToolCallRequest;
pub use na_runtime::{CompletionResponse, Protocol, Session};
