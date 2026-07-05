//! Run a shell command, gated by the [`CommandPolicy`](na_sandbox::CommandPolicy).
//!
//! [`ShellTool`] first consults the command policy: a [`Deny`](na_sandbox::Decision::Deny)
//! verdict refuses the command *without executing it*; an [`Ask`](na_sandbox::Decision::Ask)
//! verdict defers to the [`Approver`](crate::Approver). Approved commands run via
//! `cmd /C` on Windows and `sh -c` elsewhere, in the workspace root, with the
//! wall-clock budget as a hard timeout and the cancellation token killing the
//! child on demand. stdout/stderr/exit-code are composed error-first by
//! [`OutputProcessor::process_result`].

use std::process::Stdio;

use na_common::{json, CoreError, Json, Result};
use na_sandbox::{Capability, Decision};
use tokio::process::Command;

use crate::output::OutputProcessor;
use crate::tool::{BoxFuture, ResultMeta, Tool, ToolContext, ToolResult, ToolSpec};

/// Execute a shell command under the command policy and resource budget.
#[derive(Debug, Clone, Copy, Default)]
pub struct ShellTool;

impl Tool for ShellTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "shell",
            "Run a shell command in the workspace root. Destructive commands are blocked by \
             policy. Output is captured and truncated; errors are surfaced first.",
            json!({
                "type": "object",
                "required": ["command"],
                "properties": {
                    "command": { "type": "string", "minLength": 1,
                        "description": "The full command line to run." }
                },
                "additionalProperties": false
            }),
            vec![Capability::ExecuteShell],
            true,
        )
    }

    fn execute<'a>(
        &'a self,
        args: Json,
        ctx: &'a ToolContext,
    ) -> BoxFuture<'a, Result<ToolResult>> {
        Box::pin(async move {
            let command = args
                .get("command")
                .and_then(Json::as_str)
                .ok_or_else(|| CoreError::invalid_input("missing string argument \"command\""))?;

            // 1. Command-policy gate (separate from capability permission).
            match ctx.command_policy.evaluate(command) {
                Decision::Deny => {
                    return Err(CoreError::permission_denied(format!(
                        "command blocked by policy: {command}"
                    )));
                }
                Decision::Ask => {
                    if !ctx.approver.approve(Capability::ExecuteShell, command) {
                        return Err(CoreError::permission_denied(format!(
                            "command not approved: {command}"
                        )));
                    }
                }
                Decision::Allow => {}
            }

            // 2. Build the platform shell invocation.
            let mut cmd = build_command(command);
            cmd.current_dir(ctx.jail.root())
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true);

            let child = cmd
                .spawn()
                .map_err(|e| CoreError::from(e).with_context("spawning shell command"))?;

            // 3. Run to completion under timeout + cancellation.
            let deadline = ctx.budget.wall_duration();
            let cancel = ctx.cancel.clone();

            let wait = child.wait_with_output();
            let output = tokio::select! {
                biased;
                _ = cancel.cancelled() => {
                    // Child is killed on drop (kill_on_drop) when `wait` is dropped.
                    return Err(CoreError::cancelled(format!(
                        "shell command cancelled: {command}"
                    )));
                }
                res = tokio::time::timeout(deadline, wait) => {
                    match res {
                        Ok(Ok(out)) => out,
                        Ok(Err(e)) => {
                            return Err(CoreError::from(e)
                                .with_context("waiting on shell command"));
                        }
                        Err(_elapsed) => {
                            return Err(CoreError::timeout(format!(
                                "shell command exceeded {} ms: {command}",
                                ctx.budget.max_wall_ms
                            )));
                        }
                    }
                }
            };

            let exit_code = output.status.code().unwrap_or(-1);
            let processed = OutputProcessor::default().process_result(
                &output.stdout,
                &output.stderr,
                exit_code,
            );

            let ok = exit_code == 0;
            let meta = ResultMeta {
                bytes: processed.bytes,
                truncated: processed.truncated,
                was_binary: processed.was_binary,
                redactions: processed.redactions,
                untrusted: false,
                duration_ms: 0,
            };
            Ok(ToolResult {
                ok,
                content: processed.text,
                data: json!({
                    "command": command,
                    "exit_code": exit_code,
                    "stdout_bytes": output.stdout.len(),
                    "stderr_bytes": output.stderr.len(),
                }),
                summary: Some(format!("`{command}` exited {exit_code}")),
                metadata: meta,
            })
        })
    }
}

/// Build the platform-appropriate shell command.
#[cfg(windows)]
fn build_command(command: &str) -> Command {
    let mut cmd = Command::new("cmd");
    cmd.arg("/C").arg(command);
    cmd
}

/// Build the platform-appropriate shell command.
#[cfg(not(windows))]
fn build_command(command: &str) -> Command {
    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(command);
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool::{AllowAllApprover, DenyAllApprover, ToolContextBuilder};
    use na_sandbox::CommandPolicy;
    use std::sync::Arc;

    fn ctx_with(
        tag: &str,
        policy: CommandPolicy,
        approver: Arc<dyn crate::Approver>,
    ) -> ToolContext {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "na_tools_shell_{}_{}",
            tag,
            na_common::next_id("t")
        ));
        ToolContextBuilder::new(p)
            .command_policy(policy)
            .approver(approver)
            .build()
            .unwrap()
    }

    /// A cross-platform "print hello" command.
    fn echo_hello() -> &'static str {
        // `echo` exists on both cmd.exe and sh.
        "echo hello"
    }

    #[tokio::test]
    async fn runs_harmless_echo() {
        let policy = CommandPolicy::with_default(Decision::Allow);
        let c = ctx_with("echo", policy, Arc::new(AllowAllApprover));
        let res = ShellTool
            .execute(json!({ "command": echo_hello() }), &c)
            .await
            .unwrap();
        assert!(res.ok, "content: {}", res.content);
        assert!(res.content.contains("hello"));
        assert_eq!(res.data["exit_code"], 0);
    }

    #[tokio::test]
    async fn destructive_command_blocked_without_running() {
        // Default policy denies "rm -rf ...".
        let c = ctx_with("deny", CommandPolicy::default(), Arc::new(AllowAllApprover));
        let err = ShellTool
            .execute(json!({ "command": "rm -rf /" }), &c)
            .await
            .unwrap_err();
        assert!(err.is(na_common::ErrorKind::PermissionDenied));
        assert!(err.message.contains("blocked by policy"));
    }

    #[tokio::test]
    async fn ask_default_denied_by_approver() {
        // Default policy => Ask for benign commands; DenyAllApprover refuses.
        let c = ctx_with("ask", CommandPolicy::default(), Arc::new(DenyAllApprover));
        let err = ShellTool
            .execute(json!({ "command": echo_hello() }), &c)
            .await
            .unwrap_err();
        assert!(err.is(na_common::ErrorKind::PermissionDenied));
        assert!(err.message.contains("not approved"));
    }

    #[tokio::test]
    async fn nonzero_exit_is_error_first() {
        let policy = CommandPolicy::with_default(Decision::Allow);
        let c = ctx_with("fail", policy, Arc::new(AllowAllApprover));
        // `exit 3` works in both sh and cmd.
        let res = ShellTool
            .execute(json!({ "command": "exit 3" }), &c)
            .await
            .unwrap();
        assert!(!res.ok);
        assert_eq!(res.data["exit_code"], 3);
        assert!(res.content.contains("[exit 3]"));
    }

    #[tokio::test]
    async fn timeout_kills_long_command() {
        let policy = CommandPolicy::with_default(Decision::Allow);
        let mut p = std::env::temp_dir();
        p.push(format!("na_tools_shell_to_{}", na_common::next_id("t")));
        let c = ToolContextBuilder::new(p)
            .command_policy(policy)
            .approver(Arc::new(AllowAllApprover))
            .budget(na_sandbox::ResourceBudget::new(64 * 1024, 100, 50))
            .build()
            .unwrap();
        // A sleep that outlasts the 100ms budget. Use a cross-platform-ish form;
        // on Windows `ping` is the usual delay trick, on unix `sleep`.
        let cmd = if cfg!(windows) {
            "ping -n 5 127.0.0.1 >NUL"
        } else {
            "sleep 5"
        };
        let err = ShellTool
            .execute(json!({ "command": cmd }), &c)
            .await
            .unwrap_err();
        assert!(err.is(na_common::ErrorKind::Timeout), "{err}");
    }
}
