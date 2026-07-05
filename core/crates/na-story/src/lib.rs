//! `na-story` — story state management and consistency checking for the Novel Generate Agent.
//!
//! This crate provides structured state management for long-form fiction writing,
//! ensuring that the agent maintains consistency of character traits, plot constraints,
//! foreshadowing, and knowledge states across multiple chapters.
//!
//! # Core Components
//!
//! * **State Management** ([`state`]) — [`StoryState`] and related structures for
//!   tracking characters, constraints, foreshadowing, timeline, and knowledge matrix.
//!
//! * **State Manager** ([`manager`]) — [`StoryStateManager`] for loading, saving, and
//!   querying story state with atomic file operations.
//!
//! * **Prompts** ([`prompts`]) — Functions to render story state into system messages
//!   that are injected into the agent's context before generation.
//!
//! * **Consistency Guard** ([`guard`]) — [`ConsistencyGuard`] for checking generated
//!   content against character traits and constraints.

#![forbid(unsafe_code)]

pub mod guard;
pub mod helpers;
pub mod manager;
pub mod prompts;
pub mod state;

// Re-export core types for ergonomic access
pub use guard::{ConsistencyGuard, ConsistencyIssue, ConsistencyReport, IssueCategory};
pub use manager::{ContextPackage, StoryStateManager};
pub use prompts::render_state_sync_prompt;
pub use state::{
    CharacterId, CharacterState, Constraint, FactId, ForeshadowStatus, ForeshadowTracker,
    KnowledgeEntry, KnowledgeMatrix, Preference, Severity, StoryMeta, StoryState, Timeline,
    TimelineEvent, WorldRule, WorldState,
};

// Re-export common types callers will need
pub use na_common::{CoreError, Result};
