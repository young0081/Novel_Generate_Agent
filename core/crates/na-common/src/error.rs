//! Normalized error type shared across every layer of the core.
//!
//! Every failure in the system — IO, permission denial, sandbox escape, a model
//! provider hiccup, a malformed ReAct block — is funneled into a single
//! [`CoreError`] with a stable machine-readable [`ErrorKind`]/`code`. This is the
//! "error normalization" requirement: callers (and the GUI, and the model) only
//! ever have to reason about one error shape.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Stable, machine-readable classification of a failure.
///
/// The variants are intentionally coarse so that UI and the agent loop can make
/// decisions (retry? ask the user? abort the goal?) without parsing free text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// Arguments failed schema validation or were otherwise malformed.
    InvalidInput,
    /// An operation was forbidden by the permission policy.
    PermissionDenied,
    /// A path or command tried to escape the sandbox.
    SandboxViolation,
    /// A requested resource (file, tool, memory entry, checkpoint) does not exist.
    NotFound,
    /// The operation exceeded its time limit.
    Timeout,
    /// The user (or a parent) cancelled the operation.
    Cancelled,
    /// A resource budget (bytes / steps / wall-clock) was exhausted.
    BudgetExceeded,
    /// An underlying IO operation failed.
    Io,
    /// (De)serialization or parsing failed.
    Serialization,
    /// A tool reported a domain-specific failure during execution.
    Tool,
    /// The model provider failed (network, quota, bad response).
    Model,
    /// A protocol payload (ReAct block, tool-call JSON) was malformed.
    Protocol,
    /// A security guard blocked the operation (e.g. prompt injection).
    SecurityBlocked,
    /// A precondition conflict (anchor not unique, stale checkpoint, ...).
    Conflict,
    /// The agent/goal loop guard tripped (too many steps, no progress, ...).
    LoopGuard,
    /// A bug or unexpected internal state.
    Internal,
}

impl ErrorKind {
    /// Stable lowercase string code, e.g. `"permission_denied"`.
    pub fn code(self) -> &'static str {
        match self {
            ErrorKind::InvalidInput => "invalid_input",
            ErrorKind::PermissionDenied => "permission_denied",
            ErrorKind::SandboxViolation => "sandbox_violation",
            ErrorKind::NotFound => "not_found",
            ErrorKind::Timeout => "timeout",
            ErrorKind::Cancelled => "cancelled",
            ErrorKind::BudgetExceeded => "budget_exceeded",
            ErrorKind::Io => "io",
            ErrorKind::Serialization => "serialization",
            ErrorKind::Tool => "tool",
            ErrorKind::Model => "model",
            ErrorKind::Protocol => "protocol",
            ErrorKind::SecurityBlocked => "security_blocked",
            ErrorKind::Conflict => "conflict",
            ErrorKind::LoopGuard => "loop_guard",
            ErrorKind::Internal => "internal",
        }
    }

    /// Whether a blind retry of the same operation could plausibly succeed.
    pub fn default_retryable(self) -> bool {
        matches!(self, ErrorKind::Timeout | ErrorKind::Io | ErrorKind::Model)
    }
}

/// The single normalized error type used everywhere in the core.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoreError {
    pub kind: ErrorKind,
    /// Stable machine code (mirrors `kind.code()` by default).
    pub code: String,
    /// Human-readable, model-readable message.
    pub message: String,
    /// Optional extra context appended as the error bubbles up.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    /// Hint to the agent loop / UI whether retrying may help.
    pub retryable: bool,
}

impl CoreError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        CoreError {
            kind,
            code: kind.code().to_string(),
            message: message.into(),
            context: None,
            retryable: kind.default_retryable(),
        }
    }

    /// Attach (or extend) human/debug context without losing the original message.
    pub fn with_context(mut self, ctx: impl Into<String>) -> Self {
        let ctx = ctx.into();
        self.context = Some(match self.context.take() {
            Some(existing) => format!("{existing}: {ctx}"),
            None => ctx,
        });
        self
    }

    /// Force the retryable hint (overriding the kind default).
    pub fn retryable(mut self, yes: bool) -> Self {
        self.retryable = yes;
        self
    }

    pub fn is(&self, kind: ErrorKind) -> bool {
        self.kind == kind
    }

    // ----- ergonomic constructors, one per common kind -----

    pub fn invalid_input(msg: impl Into<String>) -> Self {
        Self::new(ErrorKind::InvalidInput, msg)
    }
    pub fn permission_denied(msg: impl Into<String>) -> Self {
        Self::new(ErrorKind::PermissionDenied, msg)
    }
    pub fn sandbox(msg: impl Into<String>) -> Self {
        Self::new(ErrorKind::SandboxViolation, msg)
    }
    pub fn not_found(msg: impl Into<String>) -> Self {
        Self::new(ErrorKind::NotFound, msg)
    }
    pub fn timeout(msg: impl Into<String>) -> Self {
        Self::new(ErrorKind::Timeout, msg)
    }
    pub fn cancelled(msg: impl Into<String>) -> Self {
        Self::new(ErrorKind::Cancelled, msg)
    }
    pub fn budget(msg: impl Into<String>) -> Self {
        Self::new(ErrorKind::BudgetExceeded, msg)
    }
    pub fn tool(msg: impl Into<String>) -> Self {
        Self::new(ErrorKind::Tool, msg)
    }
    pub fn model(msg: impl Into<String>) -> Self {
        Self::new(ErrorKind::Model, msg)
    }
    pub fn protocol(msg: impl Into<String>) -> Self {
        Self::new(ErrorKind::Protocol, msg)
    }
    pub fn security(msg: impl Into<String>) -> Self {
        Self::new(ErrorKind::SecurityBlocked, msg)
    }
    pub fn conflict(msg: impl Into<String>) -> Self {
        Self::new(ErrorKind::Conflict, msg)
    }
    pub fn loop_guard(msg: impl Into<String>) -> Self {
        Self::new(ErrorKind::LoopGuard, msg)
    }
    pub fn internal(msg: impl Into<String>) -> Self {
        Self::new(ErrorKind::Internal, msg)
    }
}

impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)?;
        if let Some(ctx) = &self.context {
            write!(f, " ({ctx})")?;
        }
        Ok(())
    }
}

impl std::error::Error for CoreError {}

// ----- normalization: collapse foreign error types into CoreError -----

impl From<std::io::Error> for CoreError {
    fn from(e: std::io::Error) -> Self {
        use std::io::ErrorKind as Io;
        let kind = match e.kind() {
            Io::NotFound => ErrorKind::NotFound,
            Io::PermissionDenied => ErrorKind::PermissionDenied,
            Io::TimedOut => ErrorKind::Timeout,
            _ => ErrorKind::Io,
        };
        CoreError::new(kind, e.to_string())
    }
}

impl From<serde_json::Error> for CoreError {
    fn from(e: serde_json::Error) -> Self {
        CoreError::new(ErrorKind::Serialization, e.to_string())
    }
}

impl From<tokio::time::error::Elapsed> for CoreError {
    fn from(_: tokio::time::error::Elapsed) -> Self {
        CoreError::timeout("operation timed out")
    }
}

/// Convenient result alias used throughout the workspace.
pub type Result<T> = std::result::Result<T, CoreError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_matches_kind() {
        let e = CoreError::permission_denied("nope");
        assert_eq!(e.kind, ErrorKind::PermissionDenied);
        assert_eq!(e.code, "permission_denied");
        assert!(!e.retryable);
    }

    #[test]
    fn context_chains() {
        let e = CoreError::tool("boom")
            .with_context("while writing chapter")
            .with_context("session abc");
        assert_eq!(
            e.context.as_deref(),
            Some("while writing chapter: session abc")
        );
        assert!(format!("{e}").contains("[tool] boom"));
    }

    #[test]
    fn io_not_found_normalizes() {
        let io = std::io::Error::new(std::io::ErrorKind::NotFound, "x");
        let e: CoreError = io.into();
        assert_eq!(e.kind, ErrorKind::NotFound);
    }

    #[test]
    fn timeout_is_retryable_by_default() {
        assert!(CoreError::timeout("t").retryable);
        assert!(!CoreError::invalid_input("i").retryable);
    }

    #[test]
    fn serializes_to_json() {
        let e = CoreError::not_found("file gone");
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["code"], "not_found");
        assert_eq!(v["kind"], "not_found");
    }
}
