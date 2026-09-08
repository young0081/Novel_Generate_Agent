//! Reusable, structured writing-style profiles for long-form projects.
//!
//! A profile deliberately stores rules rather than copying a source article.
//! This gives the model stable constraints it can apply to every chapter while
//! keeping the author's source material out of the generated prompt.

use serde::{Deserialize, Serialize};

/// A writing voice distilled from one or more reference articles.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StyleProfile {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub tone: String,
    #[serde(default)]
    pub narrative_person: String,
    #[serde(default)]
    pub pacing: String,
    #[serde(default)]
    pub sentence_patterns: Vec<String>,
    #[serde(default)]
    pub dialogue_rules: Vec<String>,
    #[serde(default)]
    pub imagery_rules: Vec<String>,
    #[serde(default)]
    pub banned_patterns: Vec<String>,
    #[serde(default)]
    pub humanizer_rules: Vec<String>,
    #[serde(default)]
    pub sample_excerpts: Vec<String>,
    #[serde(default)]
    pub source_article_count: u32,
    #[serde(default)]
    pub created_ms: u64,
    #[serde(default)]
    pub updated_ms: u64,
}

impl StyleProfile {
    /// Return a compact system prompt used for every planning/writing run.
    pub fn system_prompt(&self) -> String {
        let mut out = format!("# 当前选用文风：{}\n", self.name.trim());
        if !self.description.trim().is_empty() {
            out.push_str(&format!("\n风格概述：{}\n", self.description.trim()));
        }
        push_field(&mut out, "基调与情绪", &self.tone);
        push_field(&mut out, "叙事人称", &self.narrative_person);
        push_field(&mut out, "节奏", &self.pacing);
        push_rules(&mut out, "句式与段落规则", &self.sentence_patterns);
        push_rules(&mut out, "对话规则", &self.dialogue_rules);
        push_rules(&mut out, "意象与修辞规则", &self.imagery_rules);
        push_rules(&mut out, "必须避免", &self.banned_patterns);
        push_rules(&mut out, "去 AI 味执行规则", &self.humanizer_rules);
        if !self.sample_excerpts.is_empty() {
            out.push_str("\n可参考的短句（只学习节奏与取景，不得照抄）：\n");
            for sample in self.sample_excerpts.iter().take(5) {
                if !sample.trim().is_empty() {
                    out.push_str("- ");
                    out.push_str(sample.trim());
                    out.push('\n');
                }
            }
        }
        out.push_str(
            "\n执行要求：本档案是整本书的持续约束。写作前先对齐人称、基调和节奏，\
            写作后逐项自检；若剧情需要变化，只改变场景张力，不改变底层叙述声音。\n",
        );
        out
    }
}

fn push_field(out: &mut String, label: &str, value: &str) {
    if !value.trim().is_empty() {
        out.push_str(&format!("\n{label}：{}\n", value.trim()));
    }
}

fn push_rules(out: &mut String, label: &str, rules: &[String]) {
    let rules: Vec<&str> = rules
        .iter()
        .map(String::as_str)
        .map(str::trim)
        .filter(|rule| !rule.is_empty())
        .collect();
    if rules.is_empty() {
        return;
    }
    out.push_str(&format!("\n{label}：\n"));
    for rule in rules.iter().take(24) {
        out.push_str("- ");
        out.push_str(rule);
        out.push('\n');
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_prompt_contains_rules_without_source_dump() {
        let profile = StyleProfile {
            id: "s1".into(),
            name: "冷峻白描".into(),
            tone: "克制".into(),
            narrative_person: "第三人称限知".into(),
            sentence_patterns: vec!["长短句交替".into()],
            humanizer_rules: vec!["删掉空泛升华".into()],
            ..StyleProfile::default()
        };
        let prompt = profile.system_prompt();
        assert!(prompt.contains("冷峻白描"));
        assert!(prompt.contains("长短句交替"));
        assert!(prompt.contains("删掉空泛升华"));
    }
}
