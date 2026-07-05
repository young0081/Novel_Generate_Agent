//! The built-in tool implementations.
//!
//! Each submodule provides one family of [`Tool`](crate::Tool)s:
//!
//! * [`fs`] — read / write / list files.
//! * [`edit`] — anchor / full / structured file edits.
//! * [`search`] — name-glob + content-regex workspace search.
//! * [`shell`] — policy-gated command execution.
//! * [`web`] — HTML-stripping URL fetch (untrusted output).
//! * [`mcp`] — adapter exposing a remote MCP tool as a local tool.
//! * [`git`] — a literature-friendly pure-Rust version store + its tools.
//! * [`memory_tools`] — memory and checkpoint store wrappers.

pub mod edit;
pub mod fs;
pub mod git;
pub mod mcp;
pub mod memory_tools;
pub mod search;
pub mod shell;
pub mod web;
