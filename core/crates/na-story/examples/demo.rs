//! Example binary to demonstrate story state management.

use na_story::{StoryStateManager, render_state_sync_prompt};

fn main() -> na_common::Result<()> {
    println!("=== Story State Management Demo ===\n");

    // Load example story state
    let state_path = "example_story_state.json";
    let mgr = StoryStateManager::open(state_path)?;

    println!("✅ Loaded story state: {}", mgr.state.meta.title);
    println!("   Genre: {}", mgr.state.meta.genre);
    println!("   Characters: {}", mgr.state.characters.len());
    println!("   Hard Constraints: {}", mgr.state.hard_constraints.len());
    println!("   Foreshadows: {}", mgr.state.foreshadows.len());
    println!();

    // Prepare context for chapter 1
    let ctx = mgr.prepare_context(1);
    println!("📋 Context prepared for Chapter 1:");
    println!("   Relevant Characters: {}", ctx.relevant_characters.len());
    println!("   Hard Constraints (High+): {}", ctx.hard_constraints.len());
    println!("   Pending Foreshadows: {}", ctx.pending_foreshadows.len());
    println!();

    // Render prompt
    let prompt = render_state_sync_prompt(&ctx);
    println!("📝 Generated State Sync Prompt:");
    println!("{}", "=".repeat(60));
    println!("{}", prompt);
    println!("{}", "=".repeat(60));
    println!();

    // List constraints by priority
    println!("⚠️  Hard Constraints (by priority):");
    for (i, constraint) in ctx.hard_constraints.iter().enumerate() {
        println!("   {}. [{:?}] {}", i + 1, constraint.severity, constraint.description);
    }
    println!();

    // List pending foreshadows
    println!("🌱 Pending Foreshadows:");
    for fh in &ctx.pending_foreshadows {
        println!("   - {} (planted at chapter {})", fh.description, fh.planted_at);
    }
    println!();

    println!("✅ Demo completed successfully!");

    Ok(())
}
