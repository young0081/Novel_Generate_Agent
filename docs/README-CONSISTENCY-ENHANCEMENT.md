# 小说创作 Agent 长程一致性增强 - 核心方案总结

## 🎯 问题本质

**用户痛点**: Agent 越写到后面越容易"忘设定"，导致人物OOC、逻辑矛盾、时间线混乱。

**技术根因**: 
1. 设定未持续注入上下文（依赖被动召回，失败=遗忘）
2. 无结构化剧情状态管理
3. 缺少生成前同步和生成后校验

---

## 💡 核心解决方案

### 三大支柱

```
┌─────────────────────────────────────────┐
│  1. 结构化状态管理 (StoryState)        │
│     把"应该记住的"变成数据结构         │
└─────────────────────────────────────────┘
              ↓
┌─────────────────────────────────────────┐
│  2. 主动注入机制 (ContextPackage)      │
│     每次生成前强制加载关键约束         │
└─────────────────────────────────────────┘
              ↓
┌─────────────────────────────────────────┐
│  3. 一致性守护 (ConsistencyGuard)      │
│     生成后自动检查冲突                 │
└─────────────────────────────────────────┘
```

### 工作流程

```
用户: "写第6章"
  ↓
【1】加载 StoryState
  - 角色核心特征
  - Critical/High 级别硬约束
  - 未回收伏笔
  - 最近3个事件
  ↓
【2】注入到上下文
  生成前自动追加状态同步消息
  ↓
【3】Agent 生成章节
  在"看到约束"的前提下创作
  ↓
【4】一致性检查
  检测 OOC / 知识泄露 / 逻辑矛盾
  ↓
【5】状态更新
  追加新事件、更新角色状态
```

---

## 📊 关键数据结构

### StoryState (核心)

```rust
pub struct StoryState {
    // 角色状态
    pub characters: HashMap<CharacterId, CharacterState>,
    // 硬约束（按 severity 排序，Critical 最优先）
    pub hard_constraints: Vec<Constraint>,
    // 伏笔追踪
    pub foreshadows: Vec<ForeshadowTracker>,
    // 知识矩阵（谁知道什么）
    pub knowledge_matrix: KnowledgeMatrix,
    // 时间线
    pub timeline: Timeline,
    // ...
}
```

### CharacterState

```rust
pub struct CharacterState {
    pub name: String,
    pub core_traits: Vec<String>,  // 用于 OOC 检测
    pub current_status: String,
    pub goals: Vec<String>,
}
```

### Constraint

```rust
pub struct Constraint {
    pub description: String,
    pub severity: Severity,  // Critical / High / Medium / Low
}

pub enum Severity {
    Critical = 5,  // 违反=作品崩坏
    High = 4,
    Medium = 3,
    Low = 2,
}
```

---

## 🛠️ 实现要点

### 模块组织

```
core/crates/
└── na-story/          ← 新建 crate
    ├── state.rs       // 数据结构定义
    ├── manager.rs     // StoryStateManager（加载/保存/查询）
    ├── guard.rs       // ConsistencyGuard（检查逻辑）
    └── prompts.rs     // Prompt 模板
```

### 与现有系统的关系

| 组件 | 职责 | 加载方式 |
|-----|------|---------|
| **StoryState** (新) | 核心状态，必须存在 | 每次直接加载 |
| **MemoryStore** (现有) | 补充记忆，详细描述 | BM25 检索按需召回 |
| **ProjectProfile** (现有) | 静态风格指南 | 每次加载 writer.md |

三者互补：
- StoryState: "主角不会背叛朋友"（硬约束）
- MemoryStore: "主角曾在雨夜立誓，绝不背叛帮助过他的人"（详细描述）
- ProjectProfile: "使用第三人称限制视角"（写作风格）

### Token 分配

以 8K 上下文为例：
- 系统指令：1500 token
- **StoryState 精简上下文**：800 token ← 新增
- writer.md / outline.md：500 token
- 最近对话历史：4000 token
- 保留 buffer：1200 token

**优化策略**：只注入"相关"部分，不是全量 StoryState

---

## 🚀 MVP 实现路径（5天）

### Day 1: 数据结构
- [ ] 定义 `StoryState` / `CharacterState` / `Constraint` 等
- [ ] 序列化/反序列化测试

### Day 2: 状态管理
- [ ] 实现 `StoryStateManager`
- [ ] `open()` / `save()` / `prepare_context()`

### Day 2.5: Prompt 注入
- [ ] 实现 `render_state_sync_prompt()`
- [ ] 集成到 `EnhancedGoalLoop`

### Day 3: 硬约束优先
- [ ] 确保 Critical 约束始终在上下文
- [ ] 按 severity 排序

### Day 4: 一致性检查
- [ ] 实现 `ConsistencyGuard::check()`
- [ ] 检测 OOC 和知识泄露

### Day 5: GUI
- [ ] 新增「状态管理」页面
- [ ] 角色/约束/伏笔的增删改

---

## ✅ 验收标准

### 功能测试

创建测试故事：
- 主角特征："冷静"
- 硬约束："绝不背叛朋友"
- 伏笔："师傅眼神看向北方"

写 5 章，第 5 章故意诱导："主角为了复仇，出卖了好友"

**期望结果**：
- ✅ 生成前上下文中包含"绝不背叛朋友"
- ✅ 生成后一致性检查检测到 Critical 违规
- ✅ 系统拒绝或提示修正

### 性能指标

| 指标 | 目标 |
|-----|------|
| 设定遗忘率降低 | ≥ 50% |
| OOC 发生率降低 | ≥ 70% |
| 伏笔回收率 | ≥ 80% |
| 用户满意度 | ≥ 4.0/5.0 |

---

## 🔄 迭代路线

### V1.0 (MVP - 1 个月)
- ✅ 结构化状态管理
- ✅ 主动注入关键约束
- ✅ 基础一致性检查
- ✅ GUI 状态编辑

### V1.1 (+1-2 个月)
- 自动状态提取（从章节自动更新状态）
- 自动修订功能（Critical 问题自动修正）
- 时间线可视化

### V1.2 (+ 3-4 个月)
- 多分支剧情支持
- 知识矩阵图谱
- 剧情冲突预警

### V2.0 (+ 6 个月)
- 协作模式（多人共同创作）
- 向量检索增强
- 一致性评分系统

---

## 💎 关键 Prompt 模板

### 状态同步 Prompt（生成前注入）

```markdown
# 当前剧情状态同步 (第 {N} 章)

## 核心角色当前状态
- **{角色名}**:
  - 核心特征: {特征列表}
  - 当前处境: {状态}
  - 目标: {目标}
  - 知道的秘密: {已知信息}
  - **不知道的**: {未知信息}

## ⚠️ 必须遵守的硬约束
- **[Critical]** {约束描述}
- **[High]** {约束描述}

## 🌱 未回收伏笔
- {伏笔描述} (埋于第X章)

---
**重要**: 续写时必须严格符合以上状态。
```

### 一致性检查 Prompt（生成后）

```markdown
检查章节是否与设定一致：

维度：
1. 角色一致性（是否 OOC）
2. 知识一致性（是否凭空知道信息）
3. 约束遵守（是否违反硬约束）

输出 JSON:
{
  "overall_pass": true/false,
  "issues": [
    {
      "severity": "critical/high/medium/low",
      "category": "character_ooc/knowledge_leak/constraint_violation",
      "description": "问题描述",
      "suggestion": "修改建议"
    }
  ]
}
```

---

## 📚 完整文档索引

本方案包含 4 个详细文档：

1. **story-consistency-enhancement.md** (Part 1)
   - 问题诊断
   - 总体方案
   - 数据结构设计
   - Prompt 设计
   - 执行流程

2. **story-consistency-enhancement-part2.md** (Part 2)
   - 实现建议
   - 模块组织
   - 代码示例
   - MVP 实施路径

3. **story-consistency-enhancement-part3.md** (Part 3)
   - 失败模式分析
   - 测试方案
   - 代码实现示例

4. **story-consistency-enhancement-part4.md** (Part 4)
   - UI 设计建议
   - 进阶功能设想
   - FAQ

5. **implementation-checklist.md**
   - 详细实施清单
   - 时间线
   - 验收标准

---

## 🎓 核心理念

### 设计原则

1. **渐进式**: MVP 先解决最痛的问题（硬约束遗忘）
2. **自动化**: 尽量减少手动维护，自动提取状态变化
3. **可回退**: 所有操作可撤销，状态有版本备份
4. **透明化**: 让用户看到"系统记住了什么"

### 与传统方案的区别

| 传统方案 | 本方案 |
|---------|--------|
| 依赖 Agent "记住" | 结构化存储，每次强制加载 |
| 被动召回（BM25） | 主动注入（直接加载） |
| 无校验 | 生成前同步 + 生成后检查 |
| 错误累积 | 及时发现并纠正 |

---

## 🔗 快速链接

- **开始实施**: 查看 `implementation-checklist.md`
- **理解架构**: 查看 Part 1 "总体方案"
- **看代码示例**: 查看 Part 3 "代码示例"
- **了解 UI**: 查看 Part 4 "用户界面设计"

---

## 📞 支持与反馈

如有问题或需要进一步澄清，请：
1. 查阅完整文档（4 个 part）
2. 检查 implementation-checklist
3. 运行 MVP 验证测试

**预期效果**: 1 个月内推出稳定版本，显著改善长篇创作的一致性问题。

---

**方案完成日期**: 2026/06/24
**版本**: v1.0
**状态**: 待实施

