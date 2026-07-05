## 五、执行流程设计

### 5.1 完整工作流

```
用户输入: "写第6章,主角遇到神秘老者"
    ↓
【阶段1: 状态加载与上下文准备】
├─ 1.1 加载 StoryState (从 story_state.json)
├─ 1.2 提取相关角色状态(主角、可能出场角色)
├─ 1.3 查询未回收伏笔
├─ 1.4 加载 Critical/High 级别硬约束
└─ 输出: ContextPackage (结构化上下文包)
    ↓
【阶段2: 章节规划】 (可选,用户可跳过)
├─ 输入: 用户需求 + ContextPackage
├─ Agent 调用: 使用"章节规划 Prompt"生成 ChapterPlan
├─ 输出: ChapterPlan (JSON 结构)
└─ 【用户确认点】显示规划,用户可修改
    ↓
【阶段3: 正文生成】
├─ 输入: ChapterPlan + ContextPackage
├─ Agent 调用: 注入"状态同步 Prompt" + "正文生成 Prompt"
├─ 工具调用: write_file(chapter_6.md, content)
└─ 输出: 生成的章节文本
    ↓
【阶段4: 一致性检查】(自动)
├─ 输入: 新章节 + StoryState + 硬约束
├─ Agent 调用: 使用"一致性检查 Prompt"
├─ 输出: ConsistencyReport (JSON)
└─ 判断:
    ├─ 无 Critical 问题 → 继续
    └─ 有 Critical 问题 → 进入阶段5
    ↓
【阶段5: 自动修订】(条件触发)
├─ 输入: 原文 + ConsistencyReport
├─ Agent 调用: 使用"自动修订 Prompt"
├─ 输出: 修订后文稿
└─ 覆盖写入 chapter_6.md
    ↓
【阶段6: 状态更新】(自动)
├─ 6.1 更新 Timeline (追加本章事件)
├─ 6.2 更新 KnowledgeMatrix (谁新知道了什么)
├─ 6.3 更新 ForeshadowStatus (如果回收了伏笔)
├─ 6.4 追加新约束(如果本章建立了新规则)
└─ 保存 StoryState
    ↓
完成,返回给用户
```

### 5.2 阶段详细说明

| 阶段 | 输入 | 输出 | 是否需要用户确认 | 失败处理 |
|-----|------|------|----------------|---------|
| 状态加载 | story_state.json | ContextPackage | 否 | 若文件不存在,创建空状态 |
| 章节规划 | 用户需求 + Context | ChapterPlan | **是** (可选) | 用户可直接跳过或修改 |
| 正文生成 | Plan + Context | chapter.md | 否 | Agent 失败则报错 |
| 一致性检查 | 章节 + State | ConsistencyReport | 否(自动) | 无 Critical 问题直接通过 |
| 自动修订 | 原文 + Report | 修订稿 | 否(自动) | 最多重试1次,仍失败则保留原文+警告 |
| 状态更新 | 章节内容 | 新 StoryState | 否(自动) | 更新失败不阻塞,仅记录日志 |

### 5.3 用户交互点

#### 5.3.1 初次设定阶段 (项目启动)
```
用户: /init-story

系统提示: "开始故事设定向导"

系统: "请描述你的故事概念(世界观/主要角色/核心冲突)"
用户: [输入故事设定]

系统: "正在生成初始 StoryState..."
系统调用: Agent 从用户输入提取结构化信息
系统: "已生成以下设定,请确认:"
- 世界规则: [列表]
- 主要角色: [列表]
- 核心冲突: [描述]

用户: [确认/修改]

系统: "已保存 story_state.json,可以开始创作了"
```

#### 5.3.2 日常创作阶段
```
用户: "写第3章,主角潜入禁地"

[阶段1-2 自动执行]

系统: "章节规划已生成,是否查看?(y/n/skip)"
用户: y

系统显示:
---
章节规划:
目标: 主角潜入禁地,发现师傅遗物
关键事件:
  1. 突破守卫(不杀人,符合主角原则)
  2. 找到遗物(触发伏笔 fh_001)
  3. 被长老发现,仓皇逃离
知识更新:
  - 主角得知师傅真实身份
约束检查: 全部通过
---

用户: [确认/修改规划/直接生成]

[阶段3-6 自动执行]

系统: "第3章已生成,一致性检查通过 ✓"
系统: "检测到 1 个中级建议:主角逃离时的路线描述可以更详细"
用户可选择: [接受/忽略建议]
```

---

## 六、实现建议

### 6.1 模块组织 (基于 Rust)

```
core/crates/
├── na-story/          ← 新建 crate
│   ├── src/
│   │   ├── lib.rs
│   │   ├── state.rs        // StoryState 定义与持久化
│   │   ├── manager.rs      // StoryStateManager (加载/更新/查询)
│   │   ├── guard.rs        // ConsistencyGuard (检查逻辑)
│   │   ├── extractor.rs    // 从生成内容提取状态变化
│   │   └── prompts.rs      // Prompt 模板
│   └── tests/
│       └── integration.rs
├── na-memory/         ← 增强现有
│   └── src/
│       └── priority.rs     // 新增:优先级检索(硬约束优先)
└── na-runtime/        ← 集成
    └── src/
        └── enhanced_loop.rs // 集成 story 模块的增强循环
```

### 6.2 关键类/函数设计

#### 6.2.1 StoryStateManager

```rust
// core/crates/na-story/src/manager.rs

pub struct StoryStateManager {
    state: StoryState,
    state_path: PathBuf,
}

impl StoryStateManager {
    /// 从文件加载或创建新状态
    pub fn open(path: impl AsRef<Path>) -> Result<Self>;
    
    /// 保存状态到磁盘
    pub fn save(&self) -> Result<()>;
    
    /// 为新章节生成上下文包
    pub fn prepare_context(&self, chapter_num: u32) -> ContextPackage;
    
    /// 从生成的章节内容中提取状态更新
    pub async fn extract_updates(
        &mut self,
        chapter_content: &str,
        model: &dyn ModelProvider,
    ) -> Result<StateUpdate>;
    
    /// 应用状态更新
    pub fn apply_update(&mut self, update: StateUpdate) -> Result<()>;
    
    /// 获取活跃的硬约束(按优先级排序)
    pub fn active_constraints(&self, severity_min: Severity) -> Vec<&Constraint>;
    
    /// 获取未回收伏笔
    pub fn pending_foreshadows(&self) -> Vec<&ForeshadowTracker>;
}

/// 为章节生成准备的上下文包
pub struct ContextPackage {
    pub chapter_num: u32,
    pub relevant_characters: Vec<CharacterState>,
    pub recent_events: Vec<TimelineEvent>,
    pub hard_constraints: Vec<Constraint>,
    pub pending_foreshadows: Vec<ForeshadowTracker>,
    pub chapter_goal: Option<ChapterGoal>,
}

impl ContextPackage {
    /// 生成注入到 Agent 的系统消息
    pub fn to_system_message(&self) -> Message {
        // 使用 prompts::render_state_sync() 模板
    }
}
```

#### 6.2.2 ConsistencyGuard

```rust
// core/crates/na-story/src/guard.rs

pub struct ConsistencyGuard {
    // 配置
}

impl ConsistencyGuard {
    pub fn new() -> Self;
    
    /// 检查章节一致性
    pub async fn check(
        &self,
        chapter_content: &str,
        story_state: &StoryState,
        model: &dyn ModelProvider,
    ) -> Result<ConsistencyReport>;
    
    /// 根据报告自动修订章节
    pub async fn auto_fix(
        &self,
        chapter_content: &str,
        report: &ConsistencyReport,
        model: &dyn ModelProvider,
    ) -> Result<String>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsistencyReport {
    pub overall_pass: bool,
    pub issues: Vec<ConsistencyIssue>,
    pub statistics: IssueStatistics,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsistencyIssue {
    pub severity: Severity,
    pub category: IssueCategory,
    pub description: String,
    pub location: Option<String>,
    pub suggestion: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IssueCategory {
    CharacterOOC,
    KnowledgeLeak,      // 角色不该知道的信息
    TimelineError,
    ConstraintViolation,
    LogicError,
}
```

#### 6.2.3 集成到 GoalLoop

```rust
// core/crates/na-runtime/src/enhanced_loop.rs

pub struct EnhancedGoalLoop {
    base_loop: GoalLoop,
    story_manager: Option<Arc<Mutex<StoryStateManager>>>,
    consistency_guard: Option<Arc<ConsistencyGuard>>,
}

impl EnhancedGoalLoop {
    /// 带剧情状态管理的 run
    pub async fn run_with_story(
        &self,
        goal: &str,
        session: &mut Session,
        model: &dyn ModelProvider,
        registry: &ToolRegistry,
        ctx: &ToolContext,
    ) -> Result<EnhancedLoopOutcome> {
        // 1. 加载剧情状态
        let mut mgr = self.story_manager.as_ref().unwrap().lock().await;
        let context_pkg = mgr.prepare_context(/* 当前章节号 */);
        
        // 2. 注入状态同步消息到 session
        session.push(context_pkg.to_system_message());
        
        // 3. 运行基础 loop
        let outcome = self.base_loop.run(goal, session, model, registry, ctx).await?;
        
        // 4. 如果成功生成,进行一致性检查
        if outcome.stopped_reason.is_success() {
            if let Some(guard) = &self.consistency_guard {
                let content = /* 从 session 提取生成的章节 */;
                let report = guard.check(&content, &mgr.state, model).await?;
                
                // 5. 如果有 Critical 问题,自动修正
                if report.has_critical_issues() {
                    let fixed = guard.auto_fix(&content, &report, model).await?;
                    // 更新文件
                }
                
                // 6. 提取并应用状态更新
                let update = mgr.extract_updates(&content, model).await?;
                mgr.apply_update(update)?;
                mgr.save()?;
                
                return Ok(EnhancedLoopOutcome {
                    base_outcome: outcome,
                    consistency_report: Some(report),
                    state_updated: true,
                });
            }
        }
        
        Ok(EnhancedLoopOutcome {
            base_outcome: outcome,
            consistency_report: None,
            state_updated: false,
        })
    }
}
```

### 6.3 状态存储方案

**存储位置**: `workspace/story_state.json`

**持久化策略**:
- 每次章节生成后自动保存
- 使用原子写入(写临时文件→重命名)
- 保留最近3个版本作为备份(`story_state.json.1`, `.2`, `.3`)

**与现有 MemoryStore 的关系**:
- `MemoryStore`: 松散的、可召回的记忆碎片(BM25 检索)
- `StoryState`: 结构化的、必须存在的核心状态(直接加载)
- 两者互补:约束/角色核心特征→StoryState;详细设定描述→MemoryStore

### 6.4 避免 Token 爆炸的策略

#### 问题:每次都把完整 StoryState 塞进上下文 → token 太多

#### 解决方案:分层与懒加载

```rust
impl StoryStateManager {
    /// 生成"精简版"上下文(仅必要信息)
    pub fn prepare_context_compact(&self, chapter_num: u32) -> ContextPackage {
        ContextPackage {
            // 只包含本章会出场的角色
            relevant_characters: self.filter_relevant_characters(chapter_num),
            // 只包含最近3个事件
            recent_events: self.state.timeline.events.iter().rev().take(3).cloned().collect(),
            // 只包含 Critical/High 约束
            hard_constraints: self.active_constraints(Severity::High),
            // 只包含未回收的伏笔
            pending_foreshadows: self.pending_foreshadows(),
            // 本章目标
            chapter_goal: self.state.current_chapter_goal.clone(),
        }
    }
    
    fn filter_relevant_characters(&self, chapter_num: u32) -> Vec<CharacterState> {
        // 启发式:最近3章出现过的角色 + 主角
        // TODO: 后续可让 Agent 主动调用工具查询其他角色
        todo!()
    }
}
```

#### Token 预算分配(以 8K 上下文为例)

| 部分 | Token 预算 | 说明 |
|-----|-----------|------|
| System Prompt (工具目录等) | ~1500 | 基础指令 |
| StoryState 精简上下文 | ~800 | 仅关键信息 |
| writer.md / outline.md | ~500 | 风格指南 |
| 最近对话历史 | ~4000 | ContextManager.window() |
| 保留 buffer | ~1200 | 生成空间 |

**关键优化**:
1. StoryState 只注入"相关"部分,不是全量
2. 详细设定放 MemoryStore,让 Agent 用 `memory_recall` 主动查
3. 时间线只保留最近事件,更早的压缩成摘要

---

## 七、最小可行版本 (MVP)

### 7.1 MVP 范围

**第一阶段必须做** (优先级 P0):

| 功能 | 说明 | 预计工作量 |
|-----|------|-----------|
| StoryState 数据结构 | 定义核心结构(角色/约束/伏笔/知识矩阵) | 1天 |
| StoryStateManager | 加载/保存/prepare_context | 1天 |
| 状态同步 Prompt 注入 | 生成前自动注入剧情状态到上下文 | 0.5天 |
| 硬约束优先加载 | 确保 Critical 约束始终在上下文中 | 0.5天 |
| 基础一致性检查 | 检测 OOC 和知识泄露(最常见问题) | 1天 |
| 状态手动更新界面 | 用户可通过 GUI 编辑 story_state.json | 1天 |

**总计**: 约 5 个工作日

**第二阶段可迭代** (优先级 P1):

- 自动状态提取(从生成内容中自动更新状态)
- 自动修订功能
- 章节规划阶段
- 时间线可视化
- 知识矩阵图谱

**第三阶段高级功能** (优先级 P2):

- 多分支剧情支持(What-if 场景)
- 向量检索增强(更智能的相关角色识别)
- 剧情冲突预警(提前检测潜在矛盾)
- 协作模式(多人共同维护状态)

### 7.2 MVP 实现路径

#### Step 1: 定义数据结构 (Day 1)

```bash
cd core/crates
cargo new na-story --lib
cd na-story
```

在 `src/state.rs` 中定义:
- `StoryState`
- `CharacterState`
- `Constraint`
- `ForeshadowTracker`
- `KnowledgeMatrix` (简化版,先只记录"knows: bool")

**验收标准**: 能序列化/反序列化完整的 StoryState

#### Step 2: StoryStateManager (Day 2)

实现:
- `open()` / `save()`
- `prepare_context_compact()`
- `active_constraints()`

**验收标准**: 单元测试覆盖加载/保存/查询

#### Step 3: Prompt 注入 (Day 2.5)

在 `src/prompts.rs` 实现:
- `render_state_sync_message(context: &ContextPackage) -> String`

集成到 `EnhancedGoalLoop`:
- 在 `run_with_story()` 中,生成前注入状态消息

**验收标准**: 运行时日志显示注入的消息内容正确

#### Step 4: 硬约束优先 (Day 3)

修改 `ContextPackage::to_system_message()`:
- Critical/High 约束放在消息开头
- 用醒目标记(⚠️)

可选:在 MemoryStore 中新增 `recall_by_priority()`:
- 优先返回 `importance=5` 的条目

**验收标准**: 生成时 Critical 约束始终在上下文中

#### Step 5: 基础一致性检查 (Day 4)

在 `src/guard.rs` 实现:
- `ConsistencyGuard::check()` (调用模型,用一致性检查 Prompt)
- 只检查两类问题:
  1. 角色 OOC (对比 `core_traits`)
  2. 知识泄露 (角色知道不该知道的信息)

**验收标准**: 
- 输入一个故意违反设定的章节,能检测出问题
- 返回结构化的 ConsistencyReport

#### Step 6: GUI 状态编辑 (Day 5)

在 `desktop-tauri/src/` 新增「状态」页面:
- 显示当前 StoryState (JSON 格式化显示)
- 可编辑角色列表/约束列表/伏笔列表
- 保存按钮调用 Tauri 命令更新文件

**验收标准**: 
- 用户能通过 GUI 添加/编辑硬约束
- 修改后下次生成时约束生效

### 7.3 MVP 验证方案

#### 测试场景:5章小说

**设定**:
- 主角:林惊羽(冷静、重情义)
- 硬约束:"林惊羽绝不会背叛朋友"
- 伏笔:师傅临终眼神看向北方

**测试步骤**:
1. 初始化 StoryState(手动创建或通过 GUI)
2. 写第1章:师傅被杀
3. 写第2-4章:主角查线索
4. **写第5章,故意在 prompt 中诱导违反约束**:"主角为了复仇,出卖了帮助过他的好友"
5. 检查系统是否:
   - ✅ 在生成前注入了约束"绝不背叛朋友"
   - ✅ 生成后一致性检查检测到 Critical 违规
   - ✅ 拒绝或修正了这段内容

#### 成功标准

- [ ] 5章内容生成过程中,硬约束始终在上下文
- [ ] 主角行为符合"冷静、重情义"特征
- [ ] 第5章诱导测试,系统正确拒绝违规内容
- [ ] 伏笔"眼神看向北方"在状态中追踪,未被遗忘

---

