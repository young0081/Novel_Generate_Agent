## 八、容易被忽略的失败模式

### 8.1 常见陷阱

| 失败模式 | 表现 | 预防措施 |
|---------|------|---------|
| **状态更新遗漏** | 角色在第5章知道了秘密,但 KnowledgeMatrix 未更新,第7章又"不知道"了 | 每次生成后强制检查状态更新;提供"状态变更确认"界面 |
| **约束冲突** | 硬约束之间矛盾(例如:"主角不杀人" vs "主角必须复仇杀敌") | 添加约束时运行冲突检测;提示用户解决冲突 |
| **过度依赖 Agent** | 以为 Agent 会"自己记住",实际上必须结构化存储 | 文档明确说明:Agent 无记忆,一切靠 StoryState |
| **状态文件损坏** | JSON 格式错误导致加载失败 | 版本备份;加载时 schema 校验;提供修复工具 |
| **上下文溢出** | StoryState 太大,挤占了对话历史 | 严格控制注入内容;实施"按需加载"策略 |
| **检查误报** | 一致性检查把合理剧情判定为违规 | 设置"忽略本次检查"选项;收集误报样本优化 Prompt |
| **用户懒得维护** | 状态长期不更新,最终还是"忘设定" | 自动提取尽量减少手动;提供"状态健康度"提醒 |

### 8.2 边界情况处理

#### 8.2.1 角色"合理"的性格变化 vs OOC

**场景**: 主角经历重大打击后性格变冷,这是合理成长还是 OOC?

**解决方案**:
- `CharacterState` 增加 `arc_stages`:
  ```rust
  pub struct CharacterState {
      pub arc_stages: Vec<CharacterArc>,
  }
  
  pub struct CharacterArc {
      pub from_chapter: u32,
      pub to_chapter: Option<u32>,
      pub traits: Vec<String>,
      pub reason: String, // "第8章经历师傅之死,性格转变"
  }
  ```
- 一致性检查对比"当前阶段"的特征,而非初始特征

#### 8.2.2 "量子态"信息(薛定谔的秘密)

**场景**: 主角"可能知道"某个秘密,但不确定

**解决方案**:
- `KnowledgeEntry` 增加 `certainty`:
  ```rust
  pub struct KnowledgeEntry {
      pub knows: KnowledgeState,
  }
  
  pub enum KnowledgeState {
      DefinitelyKnows,
      LikelyKnows,    // 暗示过但未明说
      Unknown,
      DefinitelyUnknown,
  }
  ```

#### 8.2.3 伏笔"部分回收"

**场景**: 伏笔分多次暗示,不是一次性揭晓

**解决方案**:
- `ForeshadowStatus` 细化:
  ```rust
  pub enum ForeshadowStatus {
      Planted,
      Hinted { times: u8, last_chapter: u32 },
      PartiallyRevealed { percentage: u8 },
      FullyResolved,
      Abandoned,
  }
  ```

---

## 九、测试与评估方案

### 9.1 单元测试 (Rust)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn story_state_serialization_roundtrip() {
        let state = StoryState {
            // ... 构造测试数据
        };
        let json = serde_json::to_string(&state).unwrap();
        let loaded: StoryState = serde_json::from_str(&json).unwrap();
        assert_eq!(state, loaded);
    }
    
    #[test]
    fn constraint_priority_ordering() {
        let mgr = StoryStateManager::new_test();
        let constraints = mgr.active_constraints(Severity::Medium);
        // 验证按 severity 降序
        for w in constraints.windows(2) {
            assert!(w[0].severity >= w[1].severity);
        }
    }
    
    #[test]
    fn knowledge_matrix_lookup() {
        let mut matrix = KnowledgeMatrix::new();
        matrix.set_knowledge("char_001", "fact_secret", true);
        assert!(matrix.knows("char_001", "fact_secret"));
        assert!(!matrix.knows("char_002", "fact_secret"));
    }
}
```

### 9.2 集成测试

**测试场景**: "5章小说写作流程"

```rust
#[tokio::test]
async fn five_chapter_consistency_test() {
    // 1. 初始化状态
    let mut mgr = StoryStateManager::open("test_story.json").unwrap();
    mgr.state.characters.insert(/* 主角 */);
    mgr.state.hard_constraints.push(/* "不背叛朋友" */);
    
    // 2. 模拟生成5章
    for ch in 1..=5 {
        let ctx = mgr.prepare_context(ch);
        let session = /* 构造 session */;
        let outcome = enhanced_loop.run_with_story(
            &format!("写第{}章", ch),
            &mut session,
            &model,
            &registry,
            &tool_ctx,
        ).await.unwrap();
        
        // 验证约束始终存在
        assert!(session.history().iter().any(|m| 
            m.content.contains("不背叛朋友")
        ));
    }
    
    // 3. 验证状态更新
    assert_eq!(mgr.state.timeline.events.len(), 5);
}
```

### 9.3 对比测试 (A/B Test)

**设置**:
- **对照组**: 使用当前系统(无状态管理)写5章小说
- **实验组**: 使用增强系统(有状态管理)写同样的5章

**评估指标**:

| 维度 | 测量方法 | 目标 |
|-----|---------|------|
| **设定遗忘率** | 人工标注:生成内容中违反设定的次数 | 实验组 < 对照组 50% |
| **OOC 发生率** | 每章中角色行为不符次数 | 实验组 ≤ 1次/章 |
| **逻辑一致性** | 前后矛盾点数量 | 实验组减少 70% |
| **伏笔回收率** | 埋下的伏笔中,最终回收的比例 | 实验组 ≥ 80% |
| **用户满意度** | 1-5分主观评分 | 实验组 ≥ 4.0 |

### 9.4 真实用户测试

**招募**: 5-10位同人小说作者

**测试流程**:
1. 培训:讲解如何设置 StoryState
2. 创作任务:用系统写一篇10章短篇(约2万字)
3. 中期检查(第5章):记录遇到的问题
4. 完成后问卷:
   - 系统是否帮你记住设定? (1-5分)
   - 是否还需要手动纠正 Agent 的"遗忘"? (频率)
   - 最有用的功能?
   - 最需要改进的?

**成功标准**:
- ✅ 至少 80% 用户评分 ≥ 4 分
- ✅ 手动纠正频率 < 2次/章(对照组通常 5次/章)
- ✅ 至少 1 位用户愿意持续使用

---

## 十、实现代码示例

### 10.1 核心数据结构 (Rust)

```rust
// core/crates/na-story/src/state.rs

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub type CharacterId = String;
pub type FactId = String;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StoryState {
    pub meta: StoryMeta,
    pub world: WorldState,
    pub characters: HashMap<CharacterId, CharacterState>,
    pub timeline: Timeline,
    pub knowledge_matrix: KnowledgeMatrix,
    pub foreshadows: Vec<ForeshadowTracker>,
    pub hard_constraints: Vec<Constraint>,
    pub soft_preferences: Vec<Preference>,
    pub current_chapter_goal: Option<ChapterGoal>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StoryMeta {
    pub title: String,
    pub genre: String,
    pub last_chapter: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorldState {
    pub rules: Vec<WorldRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorldRule {
    pub id: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CharacterState {
    pub id: CharacterId,
    pub name: String,
    pub core_traits: Vec<String>,
    pub current_status: String,
    pub goals: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Timeline {
    pub current_chapter: u32,
    pub events: Vec<TimelineEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TimelineEvent {
    pub chapter: u32,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct KnowledgeMatrix {
    pub entries: HashMap<String, KnowledgeEntry>,
}

impl KnowledgeMatrix {
    pub fn new() -> Self {
        KnowledgeMatrix {
            entries: HashMap::new(),
        }
    }
    
    pub fn key(char_id: &str, fact_id: &str) -> String {
        format!("{}::{}", char_id, fact_id)
    }
    
    pub fn set_knowledge(&mut self, char_id: &str, fact_id: &str, knows: bool) {
        self.entries.insert(
            Self::key(char_id, fact_id),
            KnowledgeEntry {
                knows,
                learned_at: None,
            },
        );
    }
    
    pub fn knows(&self, char_id: &str, fact_id: &str) -> bool {
        self.entries
            .get(&Self::key(char_id, fact_id))
            .map(|e| e.knows)
            .unwrap_or(false)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct KnowledgeEntry {
    pub knows: bool,
    pub learned_at: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ForeshadowTracker {
    pub id: String,
    pub description: String,
    pub planted_at: u32,
    pub status: ForeshadowStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ForeshadowStatus {
    Planted,
    Hinted,
    Resolved,
    Abandoned,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Constraint {
    pub id: String,
    pub description: String,
    pub severity: Severity,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Info = 1,
    Low = 2,
    Medium = 3,
    High = 4,
    Critical = 5,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Preference {
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChapterGoal {
    pub chapter: u32,
    pub description: String,
}

impl Default for StoryState {
    fn default() -> Self {
        StoryState {
            meta: StoryMeta {
                title: String::new(),
                genre: String::new(),
                last_chapter: 0,
            },
            world: WorldState { rules: vec![] },
            characters: HashMap::new(),
            timeline: Timeline {
                current_chapter: 0,
                events: vec![],
            },
            knowledge_matrix: KnowledgeMatrix::new(),
            foreshadows: vec![],
            hard_constraints: vec![],
            soft_preferences: vec![],
            current_chapter_goal: None,
        }
    }
}
```

### 10.2 StoryStateManager

```rust
// core/crates/na-story/src/manager.rs

use crate::state::*;
use na_common::Result;
use std::fs;
use std::path::{Path, PathBuf};

pub struct StoryStateManager {
    pub state: StoryState,
    state_path: PathBuf,
}

impl StoryStateManager {
    /// 打开或创建 StoryState
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let state_path = path.as_ref().to_path_buf();
        let state = if state_path.exists() {
            let content = fs::read_to_string(&state_path)?;
            serde_json::from_str(&content)?
        } else {
            StoryState::default()
        };
        
        Ok(StoryStateManager { state, state_path })
    }
    
    /// 保存到磁盘
    pub fn save(&self) -> Result<()> {
        // 原子写入:先写临时文件,再重命名
        let tmp = self.state_path.with_extension("json.tmp");
        let content = serde_json::to_string_pretty(&self.state)?;
        fs::write(&tmp, content)?;
        fs::rename(&tmp, &self.state_path)?;
        Ok(())
    }
    
    /// 准备上下文包
    pub fn prepare_context(&self, chapter_num: u32) -> ContextPackage {
        ContextPackage {
            chapter_num,
            relevant_characters: self.state.characters.values().cloned().collect(),
            recent_events: self.state.timeline.events.iter().rev().take(3).cloned().collect(),
            hard_constraints: self.active_constraints(Severity::High),
            pending_foreshadows: self.pending_foreshadows(),
            chapter_goal: self.state.current_chapter_goal.clone(),
        }
    }
    
    /// 获取活跃的硬约束
    pub fn active_constraints(&self, min_severity: Severity) -> Vec<Constraint> {
        let mut constraints: Vec<_> = self
            .state
            .hard_constraints
            .iter()
            .filter(|c| c.severity >= min_severity)
            .cloned()
            .collect();
        constraints.sort_by(|a, b| b.severity.cmp(&a.severity));
        constraints
    }
    
    /// 获取未回收伏笔
    pub fn pending_foreshadows(&self) -> Vec<ForeshadowTracker> {
        self.state
            .foreshadows
            .iter()
            .filter(|f| matches!(f.status, ForeshadowStatus::Planted | ForeshadowStatus::Hinted))
            .cloned()
            .collect()
    }
}

/// 上下文包
pub struct ContextPackage {
    pub chapter_num: u32,
    pub relevant_characters: Vec<CharacterState>,
    pub recent_events: Vec<TimelineEvent>,
    pub hard_constraints: Vec<Constraint>,
    pub pending_foreshadows: Vec<ForeshadowTracker>,
    pub chapter_goal: Option<ChapterGoal>,
}
```

### 10.3 Prompt 渲染

```rust
// core/crates/na-story/src/prompts.rs

use crate::manager::ContextPackage;

pub fn render_state_sync_prompt(pkg: &ContextPackage) -> String {
    let mut prompt = String::new();
    
    prompt.push_str(&format!("# 当前剧情状态同步 (第 {} 章)\n\n", pkg.chapter_num));
    
    // 角色状态
    if !pkg.relevant_characters.is_empty() {
        prompt.push_str("## 核心角色当前状态\n");
        for char in &pkg.relevant_characters {
            prompt.push_str(&format!("- **{}**:\n", char.name));
            prompt.push_str(&format!("  - 核心特征: {}\n", char.core_traits.join(", ")));
            prompt.push_str(&format!("  - 当前处境: {}\n", char.current_status));
            if !char.goals.is_empty() {
                prompt.push_str(&format!("  - 目标: {}\n", char.goals.join("; ")));
            }
            prompt.push('\n');
        }
    }
    
    // 最近事件
    if !pkg.recent_events.is_empty() {
        prompt.push_str("## 时间线与已发生事件\n");
        for event in pkg.recent_events.iter().rev() {
            prompt.push_str(&format!("- 第{}章: {}\n", event.chapter, event.description));
        }
        prompt.push('\n');
    }
    
    // 硬约束
    if !pkg.hard_constraints.is_empty() {
        prompt.push_str("## ⚠️ 必须遵守的硬约束\n");
        for constraint in &pkg.hard_constraints {
            prompt.push_str(&format!("- **[{:?}]** {}\n", constraint.severity, constraint.description));
        }
        prompt.push('\n');
    }
    
    // 伏笔
    if !pkg.pending_foreshadows.is_empty() {
        prompt.push_str("## 🌱 未回收伏笔\n");
        for fh in &pkg.pending_foreshadows {
            prompt.push_str(&format!("- {} (埋于第{}章)\n", fh.description, fh.planted_at));
        }
        prompt.push('\n');
    }
    
    // 本章目标
    if let Some(goal) = &pkg.chapter_goal {
        prompt.push_str(&format!("## 本章创作目标\n\n\n", goal.description));
    }
    
    prompt.push_str("---\n");
    prompt.push_str("**重要**: 续写时必须严格符合以上状态。角色的行为必须符合其核心特征,不能凭空知道不该知道的信息。\n");
    
    prompt
}
```

---

