# 小说创作 Agent 长程一致性增强方案

## 执行摘要

**核心问题**：当前 Agent 在长篇续写时会逐渐"忘记"设定,导致人物 OOC、逻辑不一致、时间线混乱、伏笔丢失。

**根本原因**：
1. 设定和约束没有稳定地注入到每次生成的上下文
2. 缺少结构化的剧情状态管理
3. 没有生成前的状态同步和生成后的一致性检查
4. 记忆检索不够精准,关键约束未被优先加载

**解决方案**：基于现有架构,新增 **Story State Manager** (剧情状态管理器) + **Consistency Guard** (一致性守护者) 两个核心模块,通过结构化状态追踪 + 多阶段工作流 + 约束分级 + 自动校验,确保长程一致性。

**预期效果**：
- ✅ 设定和硬约束在每次生成时都稳定存在于上下文
- ✅ 剧情状态(谁知道什么、时间线、未回收伏笔)自动维护
- ✅ 生成前自动同步状态,生成后自动检查冲突
- ✅ MVP 可在 3-5 天内在现有代码库上实现

---

## 一、问题诊断

### 1.1 根因分析

| 问题表现 | 技术根因 | 当前系统的缺陷 |
|---------|---------|---------------|
| 人物 OOC | 角色设定未持续注入上下文 | `MemoryStore` 基于 BM25 召回,用户查询词不一定命中角色核心特征 |
| 逻辑不一致 | 缺少剧情状态追踪 | 没有结构化记录"谁知道什么"、"当前进度"等状态 |
| 时间线混乱 | 没有时间轴管理 | 事件时序关系未建模 |
| 伏笔失效 | 伏笔状态未跟踪 | `MemoryKind::Foreshadow` 存在,但无"已回收/未回收"状态机 |
| 约束丢失 | 硬约束与软偏好未区分 | 所有记忆混在一起,重要性(`importance`)不够细化 |

### 1.2 为什么"越写到后面越容易崩"

**上下文窗口衰减效应**：
- 早期章节:设定还在近期对话中,`ContextManager.window()` 能保留
- 后期章节:早期设定被压缩(`compress`)进记忆库,依赖 BM25 召回
- 召回失败 → 设定缺失 → 生成质量下降 → 用户纠正 → 但纠正信息又会在下次被压缩...恶性循环

**现有机制的局限**：
1. **`ProjectProfile` (writer.md/outline.md)** 是静态的,不会随剧情进展更新
2. **`MemoryStore.recall()`** 是被动的,依赖 agent 主动调用 `memory_recall` 工具
3. **上下文压缩** 把旧消息折叠成摘要,但摘要质量依赖 `HeuristicSummarizer`,可能丢失关键约束
4. **没有结构化状态** 来表示"第5章结束时,主角知道了秘密 X,但反派还不知道"

---

## 二、总体方案

### 2.1 架构设计

采用 **单 Agent + 多阶段工作流 + 状态守护** 模式,在现有 `GoalLoop` 基础上增强:

```
┌─────────────────────────────────────────────────────────────┐
│                   User Input (续写需求)                      │
└────────────────────────┬────────────────────────────────────┘
                         │
          ┌──────────────▼─────────────┐
          │  Story State Manager       │  ← 新增核心模块
          │  - 加载当前剧情状态        │
          │  - 提取相关设定/约束       │
          │  - 生成状态同步 Prompt     │
          └──────────────┬─────────────┘
                         │
          ┌──────────────▼─────────────┐
          │  Enhanced GoalLoop         │  ← 增强现有 loop
          │  + 注入结构化状态上下文    │
          │  + 多阶段生成              │
          └──────────────┬─────────────┘
                         │
          ┌──────────────▼─────────────┐
          │  Consistency Guard         │  ← 新增检查模块
          │  - 检测设定冲突            │
          │  - 检测时间线错误          │
          │  - 自动修正或提示          │
          └──────────────┬─────────────┘
                         │
          ┌──────────────▼─────────────┐
          │  State Update              │
          │  - 更新剧情状态            │
          │  - 标记伏笔回收            │
          │  - 追加新事实              │
          └────────────────────────────┘
```

### 2.2 模块职责

| 模块 | 职责 | 实现位置 |
|-----|------|---------|
| **Story State Manager** | 维护结构化剧情状态;生成前加载并注入关键约束 | `core/crates/na-story/src/state.rs` (新建) |
| **Consistency Guard** | 生成后检查冲突;对比生成内容与设定/状态 | `core/crates/na-story/src/guard.rs` (新建) |
| **Enhanced Memory** | 区分硬约束/软偏好;支持约束优先级 | 增强现有 `na-memory` |
| **Multi-Phase Workflow** | 分阶段:状态同步→规划→生成→检查→修正 | 新增 Skill: `write_chapter_staged` |

---

## 三、数据结构设计

### 3.1 Story State (剧情状态)

```rust
// core/crates/na-story/src/state.rs

/// 完整的剧情状态快照
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoryState {
    /// 元信息
    pub meta: StoryMeta,
    /// 世界观设定(不可变真理)
    pub world: WorldState,
    /// 角色状态(会变化)
    pub characters: HashMap<CharacterId, CharacterState>,
    /// 当前时间线位置
    pub timeline: Timeline,
    /// 信息掌握矩阵(谁知道什么)
    pub knowledge_matrix: KnowledgeMatrix,
    /// 伏笔跟踪
    pub foreshadows: Vec<ForeshadowTracker>,
    /// 硬约束(绝对不能违反)
    pub hard_constraints: Vec<Constraint>,
    /// 软偏好(尽量遵守)
    pub soft_preferences: Vec<Preference>,
    /// 本章目标
    pub current_chapter_goal: Option<ChapterGoal>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoryMeta {
    pub title: String,
    pub genre: String,
    pub created_at: u64,
    pub last_chapter: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldState {
    /// 世界观规则(例如:"魔法需要消耗生命力")
    pub rules: Vec<WorldRule>,
    /// 地理/场景
    pub locations: HashMap<String, Location>,
    /// 势力/组织
    pub factions: HashMap<String, Faction>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterState {
    pub id: CharacterId,
    pub name: String,
    /// 核心特征(用于 OOC 检测)
    pub core_traits: Vec<String>,
    /// 当前状态
    pub current_status: String,
    /// 目标/动机
    pub goals: Vec<String>,
    /// 关系网
    pub relationships: HashMap<CharacterId, Relationship>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Timeline {
    /// 当前时间点(章节编号或绝对时间)
    pub current_point: TimePoint,
    /// 已发生事件
    pub events: Vec<TimelineEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeMatrix {
    /// key: (character_id, fact_id), value: 该角色是否知道这个事实
    pub matrix: HashMap<(CharacterId, FactId), KnowledgeEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeEntry {
    pub knows: bool,
    pub learned_at: Option<TimePoint>,
    pub source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForeshadowTracker {
    pub id: String,
    pub description: String,
    pub planted_at: TimePoint,
    pub status: ForeshadowStatus,
    pub must_resolve_before: Option<TimePoint>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ForeshadowStatus {
    Planted,      // 已埋下
    Hinted,       // 已暗示
    Resolved,     // 已回收
    Abandoned,    // 废弃
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Constraint {
    pub id: String,
    pub description: String,
    pub constraint_type: ConstraintType,
    /// 违反此约束的严重程度
    pub severity: Severity,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConstraintType {
    /// 角色行为约束(例如:"主角绝不杀无辜")
    CharacterBehavior { character_id: CharacterId },
    /// 世界规则约束(例如:"时间不可逆")
    WorldRule,
    /// 逻辑一致性(例如:"死者不能复活(除非用禁术)")
    LogicalConsistency,
    /// 剧情进度(例如:"秘密 X 在第10章才能揭晓")
    PlotPacing,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum Severity {
    Critical = 5,  // 违反=作品崩坏
    High = 4,
    Medium = 3,
    Low = 2,
    Info = 1,
}
```

### 3.2 JSON 示例

```json
{
  "meta": {
    "title": "修仙逆袭录",
    "genre": "修仙/热血",
    "last_chapter": 5
  },
  "world": {
    "rules": [
      {
        "id": "wr_001",
        "description": "筑基期以下无法御剑飞行",
        "immutable": true
      }
    ]
  },
  "characters": {
    "char_001": {
      "name": "林惊羽",
      "core_traits": ["冷静", "不轻易承诺", "重情义"],
      "current_status": "练气九层,准备突破筑基",
      "goals": ["找到杀师仇人", "保护师妹"]
    }
  },
  "knowledge_matrix": {
    "matrix": {
      "char_001_fact_secret_x": {
        "knows": false,
        "comment": "主角第5章还不知道掌门是叛徒"
      },
      "char_002_fact_secret_x": {
        "knows": true,
        "learned_at": "chapter_3"
      }
    }
  },
  "foreshadows": [
    {
      "id": "fh_001",
      "description": "师傅临终时眼神看向北方",
      "planted_at": "chapter_1",
      "status": "Planted",
      "must_resolve_before": "chapter_15"
    }
  ],
  "hard_constraints": [
    {
      "id": "hc_001",
      "description": "林惊羽绝不会主动背叛朋友",
      "constraint_type": {"CharacterBehavior": {"character_id": "char_001"}},
      "severity": "Critical"
    }
  ]
}
```

---

## 四、Prompt 设计

### 4.1 状态同步 Prompt (生成前注入)

```markdown
# 当前剧情状态同步

你正在续写第 {chapter_num} 章。在开始创作前,请仔细阅读以下状态:

## 核心角色当前状态
{for each main character}
- **{character.name}**:
  - 核心特征: {character.core_traits}
  - 当前处境: {character.current_status}
  - 目标: {character.goals}
  - 知道的秘密: {character.known_facts}
  - **不知道的**: {character.unknown_facts}

## 时间线与已发生事件
- 当前进度: {timeline.current_point}
- 最近3个重要事件:
{timeline.recent_events}

## 必须遵守的硬约束 (Critical)
{for constraint in hard_constraints where severity >= High}
⚠️ {constraint.description}

## 未回收伏笔 (需注意)
{for foreshadow in foreshadows where status != Resolved}
🌱 {foreshadow.description} (埋于 {foreshadow.planted_at})

## 本章创作目标
{current_chapter_goal.description}

---
**重要**: 续写时必须严格符合以上状态。如果剧情需要角色"突然知道"某个秘密,必须通过合理的情节让 TA 得知,而不是凭空拥有这个信息。
```

### 4.2 章节规划 Prompt

```markdown
你是一位经验丰富的小说策划师。

**任务**: 为第 {chapter_num} 章制定详细写作计划,确保剧情推进合理且符合所有设定。

**输入信息**:
- 用户需求: {user_goal}
- 当前剧情状态: {story_state_summary}
- 硬约束: {hard_constraints}
- 未回收伏笔: {active_foreshadows}

**请输出 JSON 格式的章节规划**:
```json
{
  "chapter_goal": "本章要达成的剧情目标",
  "key_events": [
    {
      "sequence": 1,
      "description": "事件描述",
      "participants": ["角色A", "角色B"],
      "outcome": "此事件后的状态变化"
    }
  ],
  "knowledge_updates": [
    {
      "character": "角色名",
      "learns": "新获得的信息",
      "how": "通过什么方式得知"
    }
  ],
  "foreshadow_actions": [
    {
      "foreshadow_id": "fh_001",
      "action": "hint/resolve/none",
      "how": "如何暗示或回收"
    }
  ],
  "constraint_check": [
    {
      "constraint_id": "hc_001",
      "compliant": true,
      "note": "符合性说明"
    }
  ]
}
```

**要求**:
1. 所有角色行为必须符合其核心特征
2. 信息传递必须有明确途径(不能角色凭空知道)
3. 标注会影响哪些硬约束
4. 如果本章回收伏笔,说明如何回收
```

### 4.3 正文生成 Prompt

```markdown
你是一位专业小说作者。

**任务**: 根据章节规划,创作第 {chapter_num} 章的正文。

**章节规划**:
{chapter_plan}

**剧情状态**:
{story_state_context}

**写作要求**:
1. 严格按照规划推进剧情
2. 角色对话和行为必须符合其性格特征
3. 如果角色需要"知道"某个信息,必须在正文中展示 TA 如何得知
4. 遵守所有硬约束
5. 保持与前文的逻辑一致性

**输出格式**: 直接输出章节正文(Markdown 格式)

开始创作:
```

### 4.4 一致性检查 Prompt

```markdown
你是一位严格的编辑和逻辑审查员。

**任务**: 检查刚生成的章节是否与设定/约束/已有剧情一致。

**检查维度**:
1. **角色一致性**: 角色行为/对话是否符合其核心特征?
2. **知识一致性**: 角色是否"凭空"知道了不该知道的信息?
3. **时间线一致性**: 事件顺序是否合理?是否有时间矛盾?
4. **约束遵守**: 是否违反任何硬约束?
5. **逻辑一致性**: 是否有"前面说A,后面变B"的矛盾?

**输入**:
- 新生成章节: {generated_chapter}
- 剧情状态: {story_state}
- 硬约束: {hard_constraints}
- 角色核心特征: {character_traits}

**输出 JSON**:
```json
{
  "overall_pass": true/false,
  "issues": [
    {
      "severity": "critical/high/medium/low",
      "category": "character_ooc/knowledge_leak/timeline_error/constraint_violation/logic_error",
      "description": "具体问题描述",
      "location": "章节中的位置(段落/行)",
      "suggestion": "修改建议"
    }
  ],
  "statistics": {
    "critical_issues": 0,
    "high_issues": 0,
    "medium_issues": 1,
    "low_issues": 2
  }
}
```

**判断标准**:
- Critical: 直接破坏设定的错误(例如:死人复活,角色性格180度转变)
- High: 严重逻辑问题(例如:角色不该知道的秘密却知道了)
- Medium: 不太合理但不致命(例如:语气略微不符)
- Low: 可以优化的细节
```

### 4.5 自动修订 Prompt

```markdown
你是一位专业的小说编辑。

**任务**: 根据一致性检查报告,修正文稿中的问题。

**原文**:
{original_chapter}

**检查报告**:
{consistency_report}

**修改要求**:
1. 修复所有 Critical 和 High 级别的问题
2. 尽量保持原文风格和情节主线
3. 修改要自然,不要生硬删改
4. 如果是"角色不该知道的信息",要么删除相关描写,要么补充"如何得知"的情节

**输出**:
```json
{
  "revised_chapter": "修改后的完整章节文本(Markdown)",
  "changes": [
    {
      "issue_id": "对应检查报告中的问题",
      "action": "deleted/rewritten/added_context",
      "diff_summary": "修改摘要"
    }
  ]
}
```
```

---

