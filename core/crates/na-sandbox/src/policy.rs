//! Capability-based permission policy and command-line safety policy.
//!
//! Two complementary gates sit in front of every privileged action the agent
//! can take:
//!
//! * [`PermissionPolicy`] answers *"may this [`Capability`] be exercised on
//!   this resource?"* — e.g. may we `WriteFile` to `book/ch1.md`? It is a list
//!   of [`Rule`]s evaluated with **last-match-wins** semantics, backed by a
//!   default [`Decision`] when nothing matches. Rules match the resource string
//!   with the crate's [`glob`](crate::glob) matcher.
//!
//! * [`CommandPolicy`] answers *"is this shell command line safe to run?"*. It
//!   has explicit allow/deny glob lists plus a default, and ships with a
//!   [`CommandPolicy::default`] that refuses obviously destructive commands
//!   (`rm -rf`, `mkfs`, fork bombs, `del /`, `shutdown`, ...).
//!
//! Every decision is one of [`Decision::Allow`], [`Decision::Ask`] (defer to a
//! human approver) or [`Decision::Deny`]. The policy layer never performs the
//! action; it only classifies it.

use serde::{Deserialize, Serialize};

use crate::glob::glob_match;

/// A privileged capability the agent might exercise.
///
/// Capabilities are coarse categories of side effect; the *resource* a rule
/// matches against (a path, a URL, a memory key, ...) is supplied separately to
/// [`PermissionPolicy::evaluate`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    /// Read the contents of a file.
    ReadFile,
    /// Create or overwrite a file.
    WriteFile,
    /// Delete a file.
    DeleteFile,
    /// Enumerate a directory.
    ListDir,
    /// Run a shell command.
    ExecuteShell,
    /// Make an outbound network request.
    NetworkAccess,
    /// Read from long-term memory.
    ReadMemory,
    /// Write to long-term memory.
    WriteMemory,
    /// Perform a mutating git operation (commit, push, ...).
    GitWrite,
}

/// The verdict for a requested action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    /// Proceed without asking.
    Allow,
    /// Defer to a human approver before proceeding.
    Ask,
    /// Refuse outright.
    Deny,
}

impl Decision {
    /// `true` only for [`Decision::Allow`].
    pub fn is_allow(self) -> bool {
        matches!(self, Decision::Allow)
    }

    /// `true` only for [`Decision::Deny`].
    pub fn is_deny(self) -> bool {
        matches!(self, Decision::Deny)
    }

    /// `true` only for [`Decision::Ask`].
    pub fn needs_approval(self) -> bool {
        matches!(self, Decision::Ask)
    }
}

/// A single permission rule: *"for this capability, resources matching this
/// glob get this decision."*
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    /// The capability this rule constrains.
    pub capability: Capability,
    /// Glob (see [`crate::glob`]) matched against the resource string.
    pub pattern: String,
    /// The decision applied when both capability and pattern match.
    pub decision: Decision,
}

impl Rule {
    /// Convenience constructor.
    pub fn new(capability: Capability, pattern: impl Into<String>, decision: Decision) -> Self {
        Rule {
            capability,
            pattern: pattern.into(),
            decision,
        }
    }

    /// Does this rule apply to `(cap, resource)`?
    fn matches(&self, cap: Capability, resource: &str) -> bool {
        self.capability == cap && glob_match(&self.pattern, resource)
    }
}

/// An ordered list of [`Rule`]s plus a fallback [`Decision`].
///
/// Evaluation is **last-match-wins**: later rules override earlier ones, so the
/// idiomatic shape is a broad rule followed by narrower exceptions. If no rule
/// matches, [`default_decision`](PermissionPolicy::default_decision) is used.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionPolicy {
    /// Rules in priority order (later wins).
    pub rules: Vec<Rule>,
    /// Fallback when no rule matches.
    pub default_decision: Decision,
}

impl PermissionPolicy {
    /// A policy that allows everything by default (no rules).
    pub fn permissive() -> Self {
        PermissionPolicy {
            rules: Vec::new(),
            default_decision: Decision::Allow,
        }
    }

    /// A policy that denies everything by default (no rules).
    pub fn restrictive() -> Self {
        PermissionPolicy {
            rules: Vec::new(),
            default_decision: Decision::Deny,
        }
    }

    /// A policy that asks for approval by default (no rules).
    pub fn ask_by_default() -> Self {
        PermissionPolicy {
            rules: Vec::new(),
            default_decision: Decision::Ask,
        }
    }

    /// Construct from an explicit default with no rules.
    pub fn with_default(default_decision: Decision) -> Self {
        PermissionPolicy {
            rules: Vec::new(),
            default_decision,
        }
    }

    /// Append an `Allow` rule (builder-style, by value).
    pub fn allow(mut self, cap: Capability, pattern: impl Into<String>) -> Self {
        self.rules.push(Rule::new(cap, pattern, Decision::Allow));
        self
    }

    /// Append a `Deny` rule (builder-style, by value).
    pub fn deny(mut self, cap: Capability, pattern: impl Into<String>) -> Self {
        self.rules.push(Rule::new(cap, pattern, Decision::Deny));
        self
    }

    /// Append an `Ask` rule (builder-style, by value).
    pub fn ask(mut self, cap: Capability, pattern: impl Into<String>) -> Self {
        self.rules.push(Rule::new(cap, pattern, Decision::Ask));
        self
    }

    /// Append an arbitrary rule in place (for programmatic construction).
    pub fn push_rule(&mut self, rule: Rule) -> &mut Self {
        self.rules.push(rule);
        self
    }

    /// Evaluate a `(capability, resource)` request.
    ///
    /// Scans the rules and keeps the **last** one that matches (so later rules
    /// override earlier ones). Falls back to the default when none match.
    pub fn evaluate(&self, cap: Capability, resource: &str) -> Decision {
        let mut decision = self.default_decision;
        for rule in &self.rules {
            if rule.matches(cap, resource) {
                decision = rule.decision;
            }
        }
        decision
    }
}

impl Default for PermissionPolicy {
    /// The default permission policy asks before doing anything privileged —
    /// the safest starting point.
    fn default() -> Self {
        Self::ask_by_default()
    }
}

/// Shell-command safety classifier.
///
/// `allow`/`deny` are glob lists matched against the full command line (and so
/// can match `argv[0]` alone via e.g. `"git*"`, or a whole line via
/// `"rm -rf*"`). `deny` is consulted first and always wins; then `allow`; then
/// the `default`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandPolicy {
    /// Globs that, when matched, permit the command.
    pub allow: Vec<String>,
    /// Globs that, when matched, forbid the command (checked first, wins).
    pub deny: Vec<String>,
    /// Verdict when neither list matches.
    pub default: Decision,
}

impl CommandPolicy {
    /// An empty policy with an explicit default and no allow/deny rules.
    pub fn with_default(default: Decision) -> Self {
        CommandPolicy {
            allow: Vec::new(),
            deny: Vec::new(),
            default,
        }
    }

    /// Add an allow glob (builder-style).
    pub fn allow(mut self, pattern: impl Into<String>) -> Self {
        self.allow.push(pattern.into());
        self
    }

    /// Add a deny glob (builder-style).
    pub fn deny(mut self, pattern: impl Into<String>) -> Self {
        self.deny.push(pattern.into());
        self
    }

    /// The built-in glob list of obviously destructive command lines that
    /// [`CommandPolicy::default`] refuses.
    ///
    /// Patterns are matched against the *normalized* command line (collapsed
    /// internal whitespace) so spacing variations like `rm   -rf` are still
    /// caught.
    pub fn destructive_patterns() -> Vec<String> {
        // NOTE: command lines frequently contain path separators (`/`), and a
        // single `*` glob will *not* cross `/`. We therefore use the globstar
        // `**` (which crosses `/`) for any wildcard that must span an argument
        // such as a path. The command line is whitespace-normalized before
        // matching (see `normalize_cmdline`).
        [
            // Recursive force-remove in its common spellings.
            "rm -rf**",
            "rm -fr**",
            "rm -r -f**",
            "rm -f -r**",
            "**rm -rf**",
            "**rm -fr**",
            "sudo rm -rf**",
            "sudo rm -fr**",
            // Disk / filesystem destroyers.
            "mkfs**",
            "**mkfs.**",
            "format **",
            "format",
            "**format c:**",
            "dd if=**of=/dev/**",
            "**>/dev/sd**",
            // Windows destructive deletes.
            "del /**",
            "**del /f**",
            "**del /q**",
            "**del /s**",
            "rmdir /s**",
            "rd /s**",
            "**rmdir /s**",
            // Fork bomb (in several spacing forms).
            ":(){**",
            ":(){ :|:& };:**",
            "**:|:&**",
            // Power / system control.
            "shutdown**",
            "reboot**",
            "halt**",
            "poweroff**",
            "init 0**",
            "init 6**",
            // Overwrite the whole disk / wipe.
            "wipefs**",
            "shred**",
            // chmod/chown the entire tree.
            "chmod -R ** /",
            "**chown -R **:** /**",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect()
    }

    /// Evaluate a command line.
    ///
    /// 1. The line is normalized (trimmed, internal whitespace collapsed).
    /// 2. If any `deny` glob matches -> [`Decision::Deny`].
    /// 3. Else if any `allow` glob matches -> [`Decision::Allow`].
    /// 4. Else the `default`.
    pub fn evaluate(&self, cmdline: &str) -> Decision {
        let norm = normalize_cmdline(cmdline);

        for pat in &self.deny {
            if glob_match(pat, &norm) {
                return Decision::Deny;
            }
        }
        for pat in &self.allow {
            if glob_match(pat, &norm) {
                return Decision::Allow;
            }
        }
        self.default
    }
}

impl Default for CommandPolicy {
    /// Deny known-destructive commands, ask about everything else.
    ///
    /// This is intentionally conservative: anything not on the destructive list
    /// still requires explicit approval rather than silently running.
    fn default() -> Self {
        CommandPolicy {
            allow: Vec::new(),
            deny: Self::destructive_patterns(),
            default: Decision::Ask,
        }
    }
}

/// Trim and collapse runs of ASCII whitespace inside a command line to single
/// spaces, so glob patterns can rely on canonical spacing.
fn normalize_cmdline(cmdline: &str) -> String {
    cmdline.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decision_serde_snake_case() {
        assert_eq!(
            serde_json::to_string(&Decision::Allow).unwrap(),
            "\"allow\""
        );
        assert_eq!(serde_json::to_string(&Decision::Ask).unwrap(), "\"ask\"");
        assert_eq!(serde_json::to_string(&Decision::Deny).unwrap(), "\"deny\"");
    }

    #[test]
    fn capability_serde_snake_case() {
        assert_eq!(
            serde_json::to_string(&Capability::WriteFile).unwrap(),
            "\"write_file\""
        );
        let c: Capability = serde_json::from_str("\"execute_shell\"").unwrap();
        assert_eq!(c, Capability::ExecuteShell);
    }

    #[test]
    fn permissive_allows_unmatched() {
        let p = PermissionPolicy::permissive();
        assert_eq!(
            p.evaluate(Capability::WriteFile, "anything"),
            Decision::Allow
        );
    }

    #[test]
    fn restrictive_denies_unmatched() {
        let p = PermissionPolicy::restrictive();
        assert_eq!(p.evaluate(Capability::ReadFile, "x"), Decision::Deny);
    }

    #[test]
    fn ask_by_default_asks_unmatched() {
        let p = PermissionPolicy::ask_by_default();
        assert_eq!(p.evaluate(Capability::GitWrite, "x"), Decision::Ask);
    }

    #[test]
    fn default_policy_asks() {
        assert_eq!(
            PermissionPolicy::default().evaluate(Capability::ReadFile, "x"),
            Decision::Ask
        );
    }

    #[test]
    fn last_match_wins() {
        // Broad allow on all of book/, then deny a secret file, then a final
        // narrower allow that wins for one specific path.
        let p = PermissionPolicy::restrictive()
            .allow(Capability::WriteFile, "book/**")
            .deny(Capability::WriteFile, "book/secret.md")
            .allow(Capability::WriteFile, "book/secret.md");

        // book/secret.md: matched by rule 1 (allow), rule 2 (deny), rule 3
        // (allow) -> last wins -> Allow.
        assert_eq!(
            p.evaluate(Capability::WriteFile, "book/secret.md"),
            Decision::Allow
        );
        // Another file only matched by the first broad allow.
        assert_eq!(
            p.evaluate(Capability::WriteFile, "book/ch1.md"),
            Decision::Allow
        );
    }

    #[test]
    fn last_match_wins_deny_overrides() {
        let p = PermissionPolicy::permissive().deny(Capability::WriteFile, "**/*.lock");
        assert_eq!(
            p.evaluate(Capability::WriteFile, "deep/dir/Cargo.lock"),
            Decision::Deny
        );
        // Different capability untouched -> default Allow.
        assert_eq!(
            p.evaluate(Capability::ReadFile, "deep/dir/Cargo.lock"),
            Decision::Allow
        );
    }

    #[test]
    fn capability_must_match_too() {
        let p = PermissionPolicy::restrictive().allow(Capability::ReadFile, "**");
        assert_eq!(
            p.evaluate(Capability::ReadFile, "any/file"),
            Decision::Allow
        );
        // Same pattern, different capability: no rule matches -> default Deny.
        assert_eq!(
            p.evaluate(Capability::WriteFile, "any/file"),
            Decision::Deny
        );
    }

    #[test]
    fn unmatched_falls_through_to_default() {
        let p = PermissionPolicy::ask_by_default().allow(Capability::ReadFile, "public/**");
        assert_eq!(p.evaluate(Capability::ReadFile, "private/x"), Decision::Ask);
        assert_eq!(
            p.evaluate(Capability::ReadFile, "public/x"),
            Decision::Allow
        );
    }

    #[test]
    fn push_rule_in_place() {
        let mut p = PermissionPolicy::restrictive();
        p.push_rule(Rule::new(Capability::ListDir, "**", Decision::Allow));
        assert_eq!(p.evaluate(Capability::ListDir, "a/b"), Decision::Allow);
    }

    #[test]
    fn policy_round_trips_json() {
        let p = PermissionPolicy::restrictive()
            .allow(Capability::ReadFile, "**")
            .deny(Capability::DeleteFile, "**");
        let s = serde_json::to_string(&p).unwrap();
        let back: PermissionPolicy = serde_json::from_str(&s).unwrap();
        assert_eq!(p, back);
    }

    // ---------- CommandPolicy ----------

    #[test]
    fn destructive_commands_denied() {
        let cp = CommandPolicy::default();
        let destructive = [
            "rm -rf /",
            "rm -rf /home/user",
            "rm   -rf   .", // extra whitespace still caught
            "sudo rm -rf /",
            "rm -fr /tmp/x",
            "mkfs.ext4 /dev/sda1",
            "format c:",
            "del /f /q C:\\Windows",
            "rmdir /s /q C:\\data",
            ":(){ :|:& };:",
            "shutdown -h now",
            "reboot",
            "dd if=/dev/zero of=/dev/sda",
            "wipefs -a /dev/sda",
            "shred -u important",
        ];
        for cmd in destructive {
            assert_eq!(
                cp.evaluate(cmd),
                Decision::Deny,
                "expected DENY for destructive command: {cmd:?}"
            );
        }
    }

    #[test]
    fn benign_commands_default_to_ask() {
        let cp = CommandPolicy::default();
        assert_eq!(cp.evaluate("ls -la"), Decision::Ask);
        assert_eq!(cp.evaluate("git status"), Decision::Ask);
        assert_eq!(cp.evaluate("cargo build"), Decision::Ask);
        // A plain, non-recursive rm is *not* on the destructive list -> Ask,
        // not auto-denied (caller can still require approval).
        assert_eq!(cp.evaluate("rm file.txt"), Decision::Ask);
    }

    #[test]
    fn allow_list_permits_specific_commands() {
        let cp = CommandPolicy::default()
            .allow("git status*")
            .allow("cargo *");
        assert_eq!(cp.evaluate("git status"), Decision::Allow);
        assert_eq!(cp.evaluate("cargo test"), Decision::Allow);
        // Still denies destructive even if an allow might also match, because
        // deny is checked first.
        let cp2 = CommandPolicy::default().allow("rm*");
        assert_eq!(cp2.evaluate("rm -rf /"), Decision::Deny);
        // But a benign rm that the allow matches is allowed.
        assert_eq!(cp2.evaluate("rm notes.txt"), Decision::Allow);
    }

    #[test]
    fn explicit_deny_beats_allow() {
        let cp = CommandPolicy::with_default(Decision::Allow)
            .allow("**")
            .deny("curl **evil.test**");
        assert_eq!(cp.evaluate("curl http://evil.test/x"), Decision::Deny);
        assert_eq!(cp.evaluate("curl http://good.test/x"), Decision::Allow);
    }

    #[test]
    fn command_policy_round_trips_json() {
        let cp = CommandPolicy::default();
        let s = serde_json::to_string(&cp).unwrap();
        let back: CommandPolicy = serde_json::from_str(&s).unwrap();
        assert_eq!(cp, back);
    }

    #[test]
    fn normalize_collapses_whitespace() {
        assert_eq!(normalize_cmdline("  rm   -rf    / "), "rm -rf /");
        assert_eq!(normalize_cmdline("a\tb\nc"), "a b c");
    }
}
