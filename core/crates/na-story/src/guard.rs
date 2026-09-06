//! Consistency guard for checking generated content against story state.

use crate::state::{Severity, StoryState};
use na_common::{CoreError, Result};
use serde::{Deserialize, Serialize};

/// Consistency guard for post-generation validation.
pub struct ConsistencyGuard {
    // Configuration (future: thresholds, etc.)
}

impl ConsistencyGuard {
    pub fn new() -> Self {
        ConsistencyGuard {}
    }

    /// Check chapter content against story state.
    ///
    /// Basic checking currently requires a model-backed evaluator. Returning a
    /// passing report without evaluating the content would be unsafe because it
    /// lets callers publish a chapter under a false consistency guarantee.
    pub fn check_basic(
        &self,
        chapter_content: &str,
        story_state: &StoryState,
    ) -> Result<ConsistencyReport> {
        if chapter_content.trim().is_empty() {
            return Err(CoreError::invalid_input(
                "chapter content must not be empty",
            ));
        }
        if story_state.hard_constraints.is_empty()
            && story_state.characters.is_empty()
            && story_state.world.rules.is_empty()
        {
            return Err(CoreError::invalid_input(
                "story state has no constraints, characters, or world rules to check",
            ));
        }
        Err(CoreError::tool(
            "model-backed consistency evaluation is not configured; chapter was not checked",
        ))
    }
}

impl Default for ConsistencyGuard {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsistencyReport {
    pub overall_pass: bool,
    pub issues: Vec<ConsistencyIssue>,
    pub statistics: IssueStatistics,
}

impl ConsistencyReport {
    pub fn has_critical_issues(&self) -> bool {
        self.statistics.critical_issues > 0
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsistencyIssue {
    pub severity: Severity,
    pub category: IssueCategory,
    pub description: String,
    pub location: Option<String>,
    pub suggestion: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum IssueCategory {
    CharacterOOC,        // Out of character
    KnowledgeLeak,       // Character knows something they shouldn't
    TimelineError,       // Timeline contradiction
    ConstraintViolation, // Hard constraint violated
    LogicError,          // General logic inconsistency
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueStatistics {
    pub critical_issues: u32,
    pub high_issues: u32,
    pub medium_issues: u32,
    pub low_issues: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consistency_guard_creates() {
        let _guard = ConsistencyGuard::new();
        // Basic instantiation test - if we reach here, construction succeeded
    }

    #[test]
    fn report_has_critical_issues() {
        let report = ConsistencyReport {
            overall_pass: false,
            issues: vec![],
            statistics: IssueStatistics {
                critical_issues: 1,
                high_issues: 0,
                medium_issues: 0,
                low_issues: 0,
            },
        };
        assert!(report.has_critical_issues());

        let report2 = ConsistencyReport {
            overall_pass: true,
            issues: vec![],
            statistics: IssueStatistics {
                critical_issues: 0,
                high_issues: 2,
                medium_issues: 0,
                low_issues: 0,
            },
        };
        assert!(!report2.has_critical_issues());
    }

    #[test]
    fn check_basic_never_reports_an_unchecked_chapter_as_passing() {
        let mut state = StoryState::default();
        state.hard_constraints.push(crate::state::Constraint {
            id: "c1".to_string(),
            description: "主角绝不背叛朋友".to_string(),
            severity: Severity::Critical,
        });

        let error = ConsistencyGuard::new()
            .check_basic("主角走进城门。", &state)
            .unwrap_err();
        assert!(error.is(na_common::ErrorKind::Tool), "{error}");
        assert!(error.message.contains("not configured"));
    }

    #[test]
    fn check_basic_rejects_empty_input() {
        let error = ConsistencyGuard::new()
            .check_basic("  ", &StoryState::default())
            .unwrap_err();
        assert!(error.is(na_common::ErrorKind::InvalidInput));
    }
}
