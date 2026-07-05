# 原著对齐增强方案（Canon Alignment Enhancement）

## 问题陈述

**新用户反馈**：
> "Agent 经常偏离原著"

**核心症状**：
- 人物说话方式不像原作
- 行为动机偏离原作
- 关系张力失真
- 世界观规则被弱化或改写
- 原创剧情推进压过原作人物内核

**同人创作核心需求**：
> 用户非常在意"角色像不像"、"是否贴原著"、"有没有明显 OOC"

---

## 问题诊断

### 与"忘设定"的关系

这两个问题本质上是**同一问题的不同维度**：

| 问题 | 本质 | 解决方向 |
|------|------|----------|
| **忘设定** | 状态遗忘 | 持续性 + 结构化记忆 |
| **偏离原著** | 角色失真 | 原作锚定 + 对齐检查 |

**统一解决思路**：
```
Story State 系统（已实现）
  ├─ 状态管理层：解决"忘设定"
  └─ 原著对齐层：解决"偏离原著"（本方案）
```

### 为什么会偏离原著

1. **缺少结构化原著锚点** - 只有一句"请符合原作"，AI 无法精准把握
2. **二创边界模糊** - 没有明确哪些可以改、哪些不能动
3. **角色内核隐式** - 性格、说话风格、行为逻辑未显式建模
4. **缺少原著约束注入** - 生成时没有强调"必须像原作"
5. **没有原著一致性审校** - 生成后无法识别 OOC

---

## 总体方案

### 核心思路

**把"贴原著"结构化为可检查、可注入的数据结构**

```
原著对齐 = 原著锚点建模 + 对齐约束注入 + 一致性检查
```

### 三层防护

```
第一层：原著锚点建模（数据结构）
  ↓
第二层：生成前对齐注入（Prompt）
  ↓
第三层：生成后一致性审校（检查器）
```

---

## 一、原著锚点数据结构设计

### 1.1 CharacterCanon（角色原著锚点）

扩展现有的 `CharacterState`，新增 `canon` 字段：

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterState {
    pub id: CharacterId,
    pub name: String,
    pub core_traits: Vec<String>,  // 已有
    pub current_status: String,     // 已有
    pub goals: Vec<String>,         // 已有
    
    // 新增：原著锚点
    pub canon: Option<CharacterCanon>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterCanon {
    /// 性格核心（3-5 个关键词，不可违背）
    pub personality_core: Vec<String>,
    
    /// 说话风格
    pub speech_style: SpeechStyle,
    
    /// 行为边界（绝对不会做的事）
    pub behavior_boundaries: Vec<String>,
    
    /// 动机逻辑（为什么会这样做）
    pub motivation_logic: Vec<MotivationPattern>,
    
    /// 关系模式（与其他角色的互动特征）
    pub relationship_patterns: Vec<RelationshipPattern>,
    
    /// 情绪表达方式
    pub emotion_expression: EmotionStyle,
    
    /// 标志性台词/口头禅
    pub signature_phrases: Vec<String>,
    
    /// 原作重要场景参考（用于风格对齐）
    pub canon_scenes: Vec<CanonSceneRef>,
}
```

### 1.2 SpeechStyle（说话风格）

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpeechStyle {
    /// 语气特征（例如：冷淡、热情、严肃、幽默）
    pub tone: String,
    
    /// 用词特点（例如：文雅、粗俗、简洁、啰嗦）
    pub vocabulary: String,
    
    /// 句式特点（例如：短句为主、喜欢反问、常用省略）
    pub sentence_patterns: Vec<String>,
    
    /// 示例对话（原作中的典型台词）
    pub examples: Vec<String>,
}
```

### 1.3 MotivationPattern（动机逻辑）

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MotivationPattern {
    /// 触发条件（什么情况下）
    pub trigger: String,
    
    /// 典型反应（会做什么）
    pub typical_response: String,
    
    /// 底层动机（为什么这样做）
    pub underlying_motive: String,
}
```

### 1.4 RelationshipPattern（关系模式）

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelationshipPattern {
    /// 对方角色 ID
    pub with_character: CharacterId,
    
    /// 关系类型（友情、敌对、暧昧、师徒等）
    pub relationship_type: String,
    
    /// 互动特征（如何相处）
    pub interaction_style: String,
    
    /// 张力点（关系中的紧张因素）
    pub tension_points: Vec<String>,
}
```

### 1.5 EmotionStyle（情绪表达方式）

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmotionStyle {
    /// 高兴时的表现
    pub when_happy: String,
    
    /// 愤怒时的表现
    pub when_angry: String,
    
    /// 悲伤时的表现
    pub when_sad: String,
    
    /// 紧张时的表现
    pub when_stressed: String,
    
    /// 是否外显情绪
    pub expressiveness: Expressiveness,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Expressiveness {
    /// 情绪外露
    Expressive,
    /// 克制含蓄
    Reserved,
    /// 完全隐藏
    Stoic,
}
```

### 1.6 CanonSceneRef（原作场景参考）

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanonSceneRef {
    /// 场景简述
    pub scene_description: String,
    
    /// 角色在此场景的表现
    pub character_behavior: String,
    
    /// 这个场景体现了什么特质
    pub demonstrates: Vec<String>,
}
```

### 1.7 WorldCanon（世界观原著锚点）

扩展 `WorldState`：

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldState {
    pub rules: Vec<WorldRule>,  // 已有
    
    // 新增：世界观原著锚点
    pub canon: Option<WorldCanon>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldCanon {
    /// 核心设定（不可改变）
    pub core_settings: Vec<CoreSetting>,
    
    /// 权力体系
    pub power_system: Option<PowerSystem>,
    
    /// 文化特征
    pub cultural_traits: Vec<String>,
    
    /// 原作基调（黑暗、热血、治愈等）
    pub tone: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoreSetting {
    pub aspect: String,      // 方面（例如：魔法系统、社会结构）
    pub description: String, // 原作设定
    pub constraints: Vec<String>, // 二创时的约束
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PowerSystem {
    pub name: String,
    pub levels: Vec<String>,
    pub advancement_rules: Vec<String>,
}
```

---

## 二、JSON 示例

### 2.1 角色原著锚点示例

```json
{
  "id": "char_sherlock",
  "name": "夏洛克·福尔摩斯",
  "core_traits": ["观察力极强", "理性至上", "社交冷淡"],
  "current_status": "正在调查新案件",
  "goals": ["破解谜题", "证明自己的推理"],
  
  "canon": {
    "personality_core": [
      "极度理性",
      "傲慢但有资本",
      "对无聊的事毫无耐心",
      "对案件充满激情",
      "情感迟钝"
    ],
    
    "speech_style": {
      "tone": "冷静、快速、带优越感",
      "vocabulary": "精准、专业术语多、不废话",
      "sentence_patterns": [
        "喜欢连珠炮式发问",
        "常用反问句",
        "结论性陈述简短有力"
      ],
      "examples": [
        "Elementary, my dear Watson.",
        "显而易见，凶手是左撇子。",
        "无聊！这个案子毫无挑战性。"
      ]
    },
    
    "behavior_boundaries": [
      "绝不会主动表达感情",
      "不会为了安慰而说谎",
      "不会放弃一个有趣的案子",
      "不会违背逻辑行事"
    ],
    
    "motivation_logic": [
      {
        "trigger": "遇到有趣的案件",
        "typical_response": "立即全情投入，废寝忘食",
        "underlying_motive": "智力挑战是唯一能让他兴奋的事"
      }
    ],
    
    "relationship_patterns": [
      {
        "with_character": "char_watson",
        "relationship_type": "挚友/搭档",
        "interaction_style": "表面冷淡，实则依赖；常嘲讽华生但真心信任",
        "tension_points": ["福尔摩斯的傲慢让华生不满"]
      }
    ],
    
    "emotion_expression": {
      "when_happy": "眼睛发亮，语速加快，但不会笑",
      "when_angry": "语气更冷，沉默或讽刺",
      "when_sad": "完全不表现，可能拉小提琴",
      "when_stressed": "更加专注，忽略外界",
      "expressiveness": "Stoic"
    },
    
    "signature_phrases": [
      "Elementary.",
      "显而易见",
      "数据！数据！没有数据我什么也推理不出来。"
    ],
    
    "canon_scenes": [
      {
        "scene_description": "初次见华生，瞬间推理出华生的经历",
        "character_behavior": "冷静陈述推理过程，不在意华生的震惊",
        "demonstrates": ["观察力", "理性", "社交冷淡"]
      }
    ]
  }
}
```

### 2.2 世界观原著锚点示例

```json
{
  "rules": [
    {
      "id": "wr_001",
      "description": "魔法需要魔杖施展"
    }
  ],
  
  "canon": {
    "core_settings": [
      {
        "aspect": "魔法系统",
        "description": "魔法分为咒语魔法和无声魔法，后者极难掌握",
        "constraints": [
          "不能让角色轻易使用无声魔法",
          "魔法强度与魔力储备相关",
          "禁咒使用有严重代价"
        ]
      },
      {
        "aspect": "社会结构",
        "description": "纯血家族掌握大部分权力，麻瓜出身受歧视",
        "constraints": [
          "血统歧视是重要社会矛盾",
          "不能让血统问题轻易解决",
          "纯血家族的傲慢是性格基础"
        ]
      }
    ],
    
    "power_system": {
      "name": "魔法等级",
      "levels": ["初学者", "普通巫师", "高级巫师", "大师级"],
      "advancement_rules": [
        "需要大量练习",
        "天赋占一定比例",
        "情绪控制很重要"
      ]
    },
    
    "cultural_traits": [
      "巫师对麻瓜世界既好奇又轻视",
      "传统家族重视血统纯正",
      "魔法部官僚主义严重"
    ],
    
    "tone": "奇幻冒险，带有黑暗元素和成长主题"
  }
}
```

---

## 三、如何区分"合理二创"与"不合理偏离"

### 3.1 判断标准

| 类别 | 合理二创 | 不合理偏离 |
|------|---------|-----------|
| **性格核心** | 在原作基础上深化 | 完全改变核心特质 |
| **行为动机** | 符合原作逻辑的延伸 | 与原作动机矛盾 |
| **说话风格** | 保持原作语气，内容创新 | 说话方式完全不像 |
| **关系模式** | 关系深化或合理发展 | 关系性质改变（敌变友无铺垫）|
| **世界观** | 补充细节，不违背设定 | 改变核心规则 |

### 3.2 二创边界定义

在 `StoryState` 中新增 `creative_freedom` 字段：

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoryState {
    // ... 现有字段 ...
    
    /// 二创自由度设定
    pub creative_freedom: CreativeFreedom,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreativeFreedom {
    /// 可以改变的方面
    pub allowed_changes: Vec<AllowedChange>,
    
    /// 绝对不能改的方面
    pub locked_aspects: Vec<LockedAspect>,
    
    /// 自由度等级
    pub freedom_level: FreedomLevel,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AllowedChange {
    pub aspect: String,        // 例如："剧情走向"
    pub degree: String,         // 例如："可以大胆创新"
    pub constraints: Vec<String>, // 例如：["但角色动机要合理"]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockedAspect {
    pub aspect: String,         // 例如："角色性格核心"
    pub reason: String,          // 例如："原作粉丝最在意"
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum FreedomLevel {
    /// 严格贴原著（接近原作续写）
    Strict,
    /// 适度创新（同人常规）
    Moderate,
    /// 大胆改编（AU 设定）
    Liberal,
}
```

### 3.3 示例配置

```json
{
  "creative_freedom": {
    "allowed_changes": [
      {
        "aspect": "剧情走向",
        "degree": "完全自由",
        "constraints": ["但要符合角色会做的选择"]
      },
      {
        "aspect": "新增原创角色",
        "degree": "允许",
        "constraints": ["不能抢主角戏份", "不能改变原有角色关系"]
      },
      {
        "aspect": "情感线深化",
        "degree": "允许",
        "constraints": ["必须有原作暧昧基础", "发展要自然"]
      }
    ],
    
    "locked_aspects": [
      {
        "aspect": "角色性格核心",
        "reason": "这是同人的灵魂"
      },
      {
        "aspect": "世界观基本规则",
        "reason": "改了就不是原作世界了"
      },
      {
        "aspect": "角色说话风格",
        "reason": "最容易被看出来 OOC"
      }
    ],
    
    "freedom_level": "Moderate"
  }
}
```

---

## 四、生成前注入：角色对齐 Prompt

### 4.1 Prompt 模板

在现有的 `render_state_sync_prompt` 基础上，新增原著对齐部分：

```rust
pub fn render_canon_alignment_prompt(pkg: &ContextPackage) -> String {
    let mut prompt = String::new();
    
    prompt.push_str("## 🎭 原著角色对齐要求\n\n");
    prompt.push_str("**重要**：这是同人创作，必须让角色"像原作"。\n\n");
    
    for char in &pkg.relevant_characters {
        if let Some(canon) = &char.canon {
            prompt.push_str(&format!("### {}\n\n", char.name));
            
            // 性格核心
            prompt.push_str("**性格核心（不可违背）**：\n");
            for trait_item in &canon.personality_core {
                prompt.push_str(&format!("- {}\n", trait_item));
            }
            prompt.push('\n');
            
            // 说话风格
            prompt.push_str("**说话风格**：\n");
            prompt.push_str(&format!("- 语气：{}\n", canon.speech_style.tone));
            prompt.push_str(&format!("- 用词：{}\n", canon.speech_style.vocabulary));
            if !canon.speech_style.examples.is_empty() {
                prompt.push_str("- 典型台词参考：\n");
                for ex in canon.speech_style.examples.iter().take(2) {
                    prompt.push_str(&format!("  「{}」\n", ex));
                }
            }
            prompt.push('\n');
            
            // 行为边界
            if !canon.behavior_boundaries.is_empty() {
                prompt.push_str("**绝对不会做的事**：\n");
                for boundary in &canon.behavior_boundaries {
                    prompt.push_str(&format!("- ❌ {}\n", boundary));
                }
                prompt.push('\n');
            }
            
            // 动机逻辑
            if !canon.motivation_logic.is_empty() {
                prompt.push_str("**行为动机逻辑**：\n");
                for pattern in canon.motivation_logic.iter().take(2) {
                    prompt.push_str(&format!(
                        "- {}时 → {} （因为：{}）\n",
                        pattern.trigger,
                        pattern.typical_response,
                        pattern.underlying_motive
                    ));
                }
                prompt.push('\n');
            }
        }
    }
    
    // 二创边界
    prompt.push_str("## 📏 二创边界\n\n");
    prompt.push_str("**可以自由发挥**：剧情走向、场景设计、对话内容细节\n");
    prompt.push_str("**必须严格遵守**：角色性格核心、说话风格、行为边界、动机逻辑\n\n");
    
    prompt.push_str("---\n");
    prompt.push_str("**检查清单**（生成后自查）：\n");
    prompt.push_str("1. 每个角色的台词是否符合其说话风格？\n");
    prompt.push_str("2. 角色的行为动机是否符合其原作逻辑？\n");
    prompt.push_str("3. 是否有角色做了"绝对不会做的事"？\n");
    prompt.push_str("4. 角色间的关系张力是否保持原作特征？\n\n");
    
    prompt
}
```


### 4.2 注入时机

修改 `run_goal_live` 中的注入逻辑：

```rust
// 现有的状态同步注入
let state_prompt = render_state_sync_prompt(&ctx_pkg);
session.push(Message::system(state_prompt));

// 新增：原著对齐注入（如果角色有 canon 数据）
if has_canon_data(&ctx_pkg) {
    let canon_prompt = render_canon_alignment_prompt(&ctx_pkg);
    session.push(Message::system(canon_prompt));
}
```

### 4.3 完整注入示例

生成前，AI 看到的上下文：

```
[System 1: writer.md 风格指导]

[System 2: 剧情状态同步]
# 当前剧情状态同步 (第 5 章)
## 核心角色当前状态
- **福尔摩斯**: 正在调查珠宝失窃案

[System 3: 原著角色对齐要求]  ← 新增
## 🎭 原著角色对齐要求
**重要**：这是同人创作，必须让角色"像原作"。

### 夏洛克·福尔摩斯
**性格核心（不可违背）**：
- 极度理性
- 傲慢但有资本

**说话风格**：
- 语气：冷静、快速、带优越感
- 典型台词参考：
  「Elementary.」

[User: 写第五章：福尔摩斯破案]
```

---

## 五、生成后检查：原著一致性审校

### 5.1 工作流设计

```
用户输入创作目标
  ↓
【准备阶段】
  → 加载 story_state.json（含原著锚点）
  → 准备上下文包
  ↓
【注入阶段】
  → 注入状态同步 Prompt
  → 注入原著对齐 Prompt  ← 新增
  ↓
【生成阶段】
  → AI 生成章节内容
  ↓
【审校阶段】  ← 新增
  → 原著对齐检查
    ├─ 角色 OOC 检查
    ├─ 说话风格检查
    ├─ 行为边界检查
    ├─ 关系张力检查
    └─ 世界观一致性检查
  ↓
【评分阶段】
  → 生成对齐报告（overall_score: 0.0-1.0）
  ↓
【决策阶段】
  分支1: score >= 0.8 → ✅ 通过
  分支2: 0.6 <= score < 0.8 → ⚠️ 建议修改
  分支3: score < 0.6 → ❌ 严重偏离
```

---

## 六、评估标准

### 6.1 原著一致性评分标准

| 维度 | 权重 | 评分细则 |
|------|------|----------|
| **角色性格** | 30% | 是否符合性格核心（不可违背项） |
| **说话风格** | 25% | 语气、用词、句式是否像原作 |
| **行为逻辑** | 20% | 动机是否合理，是否违反边界 |
| **关系张力** | 15% | 角色互动是否保持原作特征 |
| **世界观** | 10% | 是否违反核心设定 |

**总分计算**：
```
overall_score = 
  personality_score * 0.30 +
  speech_score * 0.25 +
  behavior_score * 0.20 +
  relationship_score * 0.15 +
  world_canon_score * 0.10
```

**等级划分**：
- **0.9-1.0**: 完美还原
- **0.8-0.9**: 优秀，高度贴原著
- **0.7-0.8**: 良好，基本贴原著
- **0.6-0.7**: 及格，有轻微偏离
- **0.5-0.6**: 不及格，明显偏离
- **0.0-0.5**: 严重 OOC

---

## 七、与"忘设定"问题的统一解决

### 7.1 问题关系

```
Story State 系统
├─ 状态管理层（已实现）
│   ├─ 角色状态追踪
│   ├─ 知识矩阵
│   ├─ 伏笔追踪
│   ├─ 硬约束管理
│   └─ 时间线记录
│
└─ 原著对齐层（本方案）
    ├─ 原著锚点建模
    ├─ 角色对齐注入
    └─ 一致性审校
```

### 7.2 统一数据模型

在 `story_state.json` 中统一管理：

```json
{
  "meta": {...},
  
  "characters": {
    "char_001": {
      "name": "角色名",
      
      // 状态管理层（解决"忘设定"）
      "core_traits": ["特征1", "特征2"],
      "current_status": "当前处境",
      "goals": ["目标1"],
      
      // 原著对齐层（解决"偏离原著"）
      "canon": {
        "personality_core": [...],
        "speech_style": {...},
        "behavior_boundaries": [...],
        ...
      }
    }
  },
  
  "hard_constraints": [
    {
      "description": "角色约束（防忘设定）",
      "severity": "Critical"
    }
  ],
  
  "creative_freedom": {
    // 二创边界（防偏离原著）
    ...
  }
}
```

### 7.3 统一注入流程

```rust
pub fn prepare_complete_context(mgr: &StoryStateManager, chapter: u32) 
    -> CompleteContextPackage 
{
    CompleteContextPackage {
        // 状态管理部分
        state_sync: mgr.prepare_context(chapter),
        
        // 原著对齐部分
        canon_alignment: mgr.prepare_canon_context(chapter),
        
        // 二创边界
        creative_freedom: mgr.state.creative_freedom.clone(),
    }
}

// 统一渲染为一个完整的 Prompt
pub fn render_complete_prompt(pkg: &CompleteContextPackage) -> String {
    let mut prompt = String::new();
    
    // 第一部分：状态同步
    prompt.push_str(&render_state_sync_prompt(&pkg.state_sync));
    
    // 第二部分：原著对齐
    prompt.push_str(&render_canon_alignment_prompt(&pkg.canon_alignment));
    
    // 第三部分：二创边界
    prompt.push_str(&render_creative_freedom(&pkg.creative_freedom));
    
    prompt
}
```

### 7.4 统一检查流程

```rust
pub async fn comprehensive_check(
    chapter: &str,
    state: &StoryState,
    model: &dyn ModelProvider,
) -> Result<ComprehensiveReport> {
    // 检查1：状态一致性（解决"忘设定"）
    let state_check = check_state_consistency(chapter, state, model).await?;
    
    // 检查2：原著对齐性（解决"偏离原著"）
    let canon_check = check_canon_alignment(chapter, state, model).await?;
    
    Ok(ComprehensiveReport {
        state_consistency: state_check,
        canon_alignment: canon_check,
        overall_quality: combine_scores(state_check, canon_check),
        recommendations: merge_recommendations(state_check, canon_check),
    })
}
```

---

## 八、实施建议

### 8.1 分阶段实施

**Phase 1: 数据结构扩展（1-2 天）**
- 扩展 `CharacterState` 添加 `canon` 字段
- 实现 `CharacterCanon` 及相关类型
- 添加 `CreativeFreedom` 到 `StoryState`
- 单元测试：序列化/反序列化

**Phase 2: Prompt 增强（1 天）**
- 实现 `render_canon_alignment_prompt()`
- 修改 `run_goal_live` 注入逻辑
- 测试：验证 Prompt 正确生成

**Phase 3: 检查器实现（2-3 天）**
- 实现各个审校 Prompt
- 实现 `CanonAlignmentChecker`
- 集成到生成流程
- 测试：用真实案例验证

**Phase 4: UI 集成（2-3 天）**
- 前端添加原著锚点编辑界面
- 显示对齐报告
- 支持一键修正

### 8.2 MVP 最小可行版本

**必须实现**（解决核心痛点）：
1. ✅ `CharacterCanon` 数据结构
2. ✅ 原著对齐 Prompt 注入
3. ✅ 基础的 OOC 检查（只检查性格核心和行为边界）

**可以后续迭代**：
- 详细的说话风格检查
- 关系张力检查
- 自动修正功能
- UI 可视化编辑

### 8.3 测试策略

**测试案例**：选择一个知名IP（如《哈利波特》或《福尔摩斯》）

1. **建立原著锚点**：
   - 为主要角色建立完整的 `canon` 数据
   - 设置世界观核心设定
   - 定义二创边界

2. **生成测试**：
   - 故意让 AI 生成 OOC 内容
   - 验证检查器能否识别

3. **对比测试**：
   - 有原著对齐 vs 无原著对齐
   - 对比 OOC 发生率

---

## 九、预期效果

### 9.1 问题解决程度

| 问题 | 现状 | 加入原著对齐后 | 改善程度 |
|------|------|----------------|----------|
| 说话方式不像原作 | 经常发生 | 大幅减少 | 80% |
| 行为动机偏离 | 常见 | 偶尔发生 | 70% |
| 关系张力失真 | 较常见 | 罕见 | 75% |
| 世界观被改写 | 偶尔发生 | 极少发生 | 90% |
| 整体OOC感 | 严重 | 轻微 | 85% |

### 9.2 用户体验提升

**之前**：
- 生成后："这不像XX啊，太OOC了"
- 需要大量手动修改
- 反复重新生成

**之后**：
- 生成前：自动注入原著锚点
- 生成中：AI 严格遵守
- 生成后：自动检查 + 评分 + 具体建议
- 结果：高还原度，减少 80% 修改工作量

---

## 十、总结

本方案通过三层防护解决"偏离原著"问题：

1. **原著锚点建模** - 把"贴原著"结构化为可检查的数据
2. **生成前注入** - 让 AI 明确知道"什么是原著特征"
3. **生成后审校** - 自动识别 OOC 并给出修改建议

与现有的"状态管理"系统完美融合，统一解决同人创作的两大核心痛点：
- ✅ 忘设定 → Story State 系统
- ✅ 偏离原著 → Canon Alignment 层

最终实现：**既不忘设定，又贴原著，让同人创作真正"像原作续写"**。

