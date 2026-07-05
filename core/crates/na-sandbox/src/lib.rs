//! `na-sandbox` — isolation, permission policy, and resource budgets.
//!
//! This crate is the safety perimeter of the Novel Generate Team core. Before
//! any tool touches the filesystem, runs a command, or spends machine
//! resources, it passes through the gates defined here:
//!
//! * [`PathJail`] — confines every filesystem path to a canonical root
//!   directory, defeating `..`, absolute-path, and Windows-drive escape
//!   attempts via purely *lexical* normalization (so not-yet-existing files and
//!   symlinks behave predictably). See [`path_jail`].
//!
//! * [`PermissionPolicy`] / [`Capability`] / [`Decision`] — a capability-based,
//!   last-match-wins rule engine that classifies a `(capability, resource)`
//!   request as [`Decision::Allow`], [`Decision::Ask`], or [`Decision::Deny`].
//!   See [`policy`].
//!
//! * [`CommandPolicy`] — a shell-command safety classifier that denies
//!   obviously destructive command lines (`rm -rf`, `mkfs`, fork bombs, …) out
//!   of the box. See [`policy`].
//!
//! * [`ResourceBudget`] / [`StepCounter`] — hard ceilings on captured output
//!   bytes, wall-clock time, and step count, surfaced as
//!   [`BudgetExceeded`](na_common::ErrorKind::BudgetExceeded) errors. See
//!   [`budget`].
//!
//! Glob matching used by the policy layer lives in [`glob`].
//!
//! Every fallible operation returns [`na_common::Result`] and every error is a
//! normalized [`na_common::CoreError`].

#![forbid(unsafe_code)]
#![warn(missing_debug_implementations)]

pub mod budget;
pub mod glob;
pub mod path_jail;
pub mod policy;

pub use budget::{ResourceBudget, StepCounter};
pub use glob::{glob_match, glob_to_regex};
pub use path_jail::PathJail;
pub use policy::{Capability, CommandPolicy, Decision, PermissionPolicy, Rule};

#[cfg(test)]
mod tests {
    //! Cross-module smoke tests exercising the public surface together.

    use super::*;

    #[test]
    fn public_reexports_are_usable() {
        // Glob.
        assert!(glob_match("a/*.rs", "a/b.rs"));

        // Permission policy.
        let policy = PermissionPolicy::restrictive()
            .allow(Capability::ReadFile, "book/**")
            .deny(Capability::ReadFile, "book/private/**");
        assert_eq!(
            policy.evaluate(Capability::ReadFile, "book/ch1.md"),
            Decision::Allow
        );
        assert_eq!(
            policy.evaluate(Capability::ReadFile, "book/private/diary.md"),
            Decision::Deny
        );

        // Command policy.
        assert_eq!(
            CommandPolicy::default().evaluate("rm -rf /"),
            Decision::Deny
        );

        // Budget + step counter.
        let budget = ResourceBudget::default();
        assert!(budget.check_bytes(10).is_ok());
        let mut steps: StepCounter = budget.step_counter();
        assert!(steps.tick().is_ok());

        // Rule construction.
        let rule = Rule::new(Capability::WriteFile, "**", Decision::Ask);
        assert_eq!(rule.decision, Decision::Ask);
    }

    #[test]
    fn path_jail_end_to_end() {
        let mut dir = std::env::temp_dir();
        dir.push(format!("na_sandbox_lib_{}", na_common::next_id("t")));
        let jail = PathJail::new(&dir).expect("jail");
        assert!(jail.resolve("notes/ch1.md").is_ok());
        assert!(jail.resolve("../../etc/passwd").is_err());
    }
}
