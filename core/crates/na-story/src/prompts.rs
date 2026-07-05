//! Prompt rendering functions for injecting story state into context.

use crate::manager::ContextPackage;

/// Render story state into a system message for injection into agent context.
pub fn render_state_sync_prompt(pkg: &ContextPackage) -> String {
    let mut prompt = String::new();

    prompt.push_str(&format!(
        "# 当前剧情状态同步 (第 {} 章)\n\n",
        pkg.chapter_num
    ));

    // Characters
    if !pkg.relevant_characters.is_empty() {
        prompt.push_str("## 核心角色当前状态\n");
        for char in &pkg.relevant_characters {
            prompt.push_str(&format!("- **{}**:\n", char.name));
            if !char.core_traits.is_empty() {
                prompt.push_str(&format!("  - 核心特征: {}\n", char.core_traits.join(", ")));
            }
            prompt.push_str(&format!("  - 当前处境: {}\n", char.current_status));
            if !char.goals.is_empty() {
                prompt.push_str(&format!("  - 目标: {}\n", char.goals.join("; ")));
            }
            prompt.push('\n');
        }
    }

    // Recent events
    if !pkg.recent_events.is_empty() {
        prompt.push_str("## 时间线与已发生事件\n");
        for event in pkg.recent_events.iter().rev() {
            prompt.push_str(&format!("- 第{}章: {}\n", event.chapter, event.description));
        }
        prompt.push('\n');
    }

    // Hard constraints
    if !pkg.hard_constraints.is_empty() {
        prompt.push_str("## ⚠️ 必须遵守的硬约束\n");
        for constraint in &pkg.hard_constraints {
            prompt.push_str(&format!(
                "- **[{:?}]** {}\n",
                constraint.severity, constraint.description
            ));
        }
        prompt.push('\n');
    }

    // Foreshadowing
    if !pkg.pending_foreshadows.is_empty() {
        prompt.push_str("## 🌱 未回收伏笔\n");
        for fh in &pkg.pending_foreshadows {
            prompt.push_str(&format!(
                "- {} (埋于第{}章)\n",
                fh.description, fh.planted_at
            ));
        }
        prompt.push('\n');
    }

    // Chapter goal
    if let Some(goal) = &pkg.chapter_goal {
        prompt.push_str(&format!("## 本章创作目标\n{}\n\n", goal.description));
    }

    prompt.push_str("---\n");
    prompt.push_str("**重要**: 续写时必须严格符合以上状态。角色的行为必须符合其核心特征,不能凭空知道不该知道的信息。\n");

    prompt
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::*;

    #[test]
    fn prompt_includes_critical_constraints() {
        let char_state = CharacterState {
            id: "c1".to_string(),
            name: "主角".to_string(),
            core_traits: vec!["冷静".to_string()],
            current_status: "初始状态".to_string(),
            goals: vec![],
        };

        let constraint = Constraint {
            id: "hc1".to_string(),
            description: "主角绝不背叛朋友".to_string(),
            severity: Severity::Critical,
        };

        let pkg = ContextPackage {
            chapter_num: 5,
            relevant_characters: vec![char_state],
            recent_events: vec![],
            hard_constraints: vec![constraint],
            pending_foreshadows: vec![],
            chapter_goal: None,
        };

        let prompt = render_state_sync_prompt(&pkg);
        assert!(prompt.contains("第 5 章"));
        assert!(prompt.contains("主角"));
        assert!(prompt.contains("冷静"));
        assert!(prompt.contains("⚠️"));
        assert!(prompt.contains("Critical"));
        assert!(prompt.contains("主角绝不背叛朋友"));
    }

    #[test]
    fn prompt_includes_foreshadowing() {
        let fh = ForeshadowTracker {
            id: "f1".to_string(),
            description: "师傅眼神看向北方".to_string(),
            planted_at: 1,
            status: ForeshadowStatus::Planted,
        };

        let pkg = ContextPackage {
            chapter_num: 5,
            relevant_characters: vec![],
            recent_events: vec![],
            hard_constraints: vec![],
            pending_foreshadows: vec![fh],
            chapter_goal: None,
        };

        let prompt = render_state_sync_prompt(&pkg);
        assert!(prompt.contains("🌱"));
        assert!(prompt.contains("师傅眼神看向北方"));
        assert!(prompt.contains("埋于第1章"));
    }

    #[test]
    fn prompt_includes_timeline() {
        let event = TimelineEvent {
            chapter: 3,
            description: "主角突破境界".to_string(),
        };

        let pkg = ContextPackage {
            chapter_num: 5,
            relevant_characters: vec![],
            recent_events: vec![event],
            hard_constraints: vec![],
            pending_foreshadows: vec![],
            chapter_goal: None,
        };

        let prompt = render_state_sync_prompt(&pkg);
        assert!(prompt.contains("时间线"));
        assert!(prompt.contains("第3章"));
        assert!(prompt.contains("主角突破境界"));
    }
}
