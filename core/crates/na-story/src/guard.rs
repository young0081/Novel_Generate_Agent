//! Consistency guard for checking generated content against story state.

use crate::state::{Severity, StoryState};
use na_common::Result;
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
    /// This is a placeholder that will be fully implemented when integrating
    /// with na-runtime. For MVP, this provides the structure and returns a
    /// basic passing report.
    pub fn check_basic(
        &self,
        _chapter_content: &str,
        _story_state: &StoryState,
    ) -> Result<ConsistencyReport> {
        // TODO: Implement actual consistency checking
        // This will be called from na-runtime with model access
        Ok(ConsistencyReport {
            overall_pass: true,
            issues: vec![],
            statistics: IssueStatistics {
                critical_issues: 0,
                high_issues: 0,
                medium_issues: 0,
                low_issues: 0,
            },
        })
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
    CharacterOOC,       // Out of character
    KnowledgeLeak,      // Character knows something they shouldn't
    TimelineError,      // Timeline contradiction
    ConstraintViolation, // Hard constraint violated
    LogicError,         // General logic inconsistency
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
}
