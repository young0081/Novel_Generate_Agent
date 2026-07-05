# 实施 Checklist

## 阶段一：基础设施 (Day 1-2)

### Day 1: 数据结构与存储

- [ ] 创建新 crate `na-story`
  ```bash
  cd core/crates
  cargo new na-story --lib
  ```

- [ ] 在 `Cargo.toml` 添加依赖
  ```toml
  [dependencies]
  na-common = { path = "../na-common" }
  na-memory = { path = "../na-memory" }
  serde = { version = "1.0", features = ["derive"] }
  serde_json = "1.0"
  ```

- [ ] 实现核心数据结构 (`src/state.rs`)
  - [ ] `StoryState`
  - [ ] `CharacterState`
  - [ ] `Constraint` + `Severity` enum
  - [ ] `ForeshadowTracker` + `ForeshadowStatus` enum
  - [ ] `Timeline` + `TimelineEvent`
  - [ ] `KnowledgeMatrix` + `KnowledgeEntry`
  - [ ] `WorldState` + `WorldRule`

- [ ] 编写序列化测试
  ```rust
  #[test]
  fn story_state_serialization_roundtrip() { ... }
  ```

### Day 2: 状态管理器

- [ ] 实现 `StoryStateManager` (`src/manager.rs`)
  - [ ] `open()` - 加载或创建
  - [ ] `save()` - 原子写入 + 备份
  - [ ] `prepare_context()` - 生成上下文包
  - [ ] `active_constraints()` - 按优先级过滤
  - [ ] `pending_foreshadows()` - 未回收伏笔

- [ ] 实现 `ContextPackage`
  - [ ] 包含必要字段
  - [ ] 实现 `to_system_message()` (暂用简单模板)

- [ ] 单元测试
  ```rust
  #[test]
  fn constraint_priority_ordering() { ... }
  
  #[test]
  fn context_package_filters_relevant_data() { ... }
  ```

---

## 阶段二：Prompt 与集成 (Day 2.5-3)

### Day 2.5: Prompt 模板

- [ ] 创建 `src/prompts.rs`

- [ ] 实现 `render_state_sync_prompt()`
  - [ ] 角色状态部分
  - [ ] 最近事件部分
  - [ ] 硬约束部分(加 ⚠️ 标记)
  - [ ] 伏笔部分
  - [ ] 本章目标部分

- [ ] 测试 Prompt 生成
  ```rust
  #[test]
  fn prompt_includes_critical_constraints() {
      let pkg = /* 构造测试包 */;
      let prompt = render_state_sync_prompt(&pkg);
      assert!(prompt.contains("⚠️"));
      assert!(prompt.contains("Critical"));
  }
  ```

### Day 3: 集成到 Runtime

- [ ] 修改 `na-runtime/Cargo.toml`
  ```toml
  [dependencies]
  na-story = { path = "../na-story" }
  ```

- [ ] 在 `na-runtime` 新增 `src/enhanced_loop.rs`
  - [ ] 定义 `EnhancedGoalLoop` 结构
  - [ ] 实现 `run_with_story()` 方法
  - [ ] 生成前注入状态消息

- [ ] 在现有 `run_goal_live` Tauri 命令中集成
  ```rust
  // 伪代码
  if let Some(state_path) = workspace.join("story_state.json") {
      let mgr = StoryStateManager::open(state_path)?;
      let ctx_pkg = mgr.prepare_context(chapter_num);
      session.push(ctx_pkg.to_system_message());
  }
  ```

- [ ] 验证:运行 demo,检查 session 中是否包含状态消息

---

## 阶段三：优先级与检查 (Day 4)

### Day 4 上午: 硬约束优先

- [ ] 确保 `active_constraints()` 按 severity 降序排序

- [ ] 在 `render_state_sync_prompt()` 中:
  - [ ] Critical 约束放在最前面
  - [ ] 用醒目格式(红色标记/加粗)

- [ ] 可选:增强 MemoryStore
  ```rust
  // na-memory/src/memory.rs
  impl MemoryStore {
      pub fn recall_by_priority(&self, query: &str, k: usize) -> Vec<RecallHit> {
          // 优先返回 importance=5 的条目
      }
  }
  ```

### Day 4 下午: 基础一致性检查

- [ ] 创建 `src/guard.rs`

- [ ] 定义数据结构
  - [ ] `ConsistencyGuard`
  - [ ] `ConsistencyReport`
  - [ ] `ConsistencyIssue`
  - [ ] `IssueCategory` enum

- [ ] 实现 `ConsistencyGuard::check()`
  - [ ] 构造检查 Prompt (见第四章)
  - [ ] 调用 `model.complete()` 获取结构化响应
  - [ ] 解析 JSON 为 `ConsistencyReport`

- [ ] 编写测试用例
  ```rust
  #[tokio::test]
  async fn detects_character_ooc() {
      let guard = ConsistencyGuard::new();
      let bad_chapter = "主角突然暴怒,大开杀戒..."; // 违反"冷静"特征
      let state = /* 构造包含"冷静"特征的状态 */;
      let report = guard.check(bad_chapter, &state, &mock_model).await?;
      
      assert!(!report.overall_pass);
      assert!(report.issues.iter().any(|i| i.category == IssueCategory::CharacterOOC));
  }
  ```

- [ ] 集成到 `EnhancedGoalLoop`
  - [ ] 生成后自动调用 `check()`
  - [ ] 如果有 Critical 问题,记录警告(MVP 不做自动修正)

---

## 阶段四：GUI 界面 (Day 5)

### Day 5: 状态管理页面

- [ ] 在 `desktop-tauri/src-tauri/src/lib.rs` 新增命令
  ```rust
  #[tauri::command]
  async fn load_story_state(workspace: String) -> Result<StoryState, String> {
      // ...
  }
  
  #[tauri::command]
  async fn save_story_state(workspace: String, state: StoryState) -> Result<(), String> {
      // ...
  }
  ```

- [ ] 前端新增路由
  - [ ] 在 `desktop-tauri/src/App.tsx` 添加 `/state` 路由
  - [ ] 创建 `src/pages/StateManagement.tsx`

- [ ] 实现 UI 组件
  - [ ] 概览卡片(当前进度/角色数/约束数)
  - [ ] 角色列表 + 编辑表单
  - [ ] 约束列表(按 severity 分组) + 编辑表单
  - [ ] 伏笔列表 + 状态切换按钮
  - [ ] 知识矩阵(简化版:只显示核心角色的已知/未知)

- [ ] 样式
  - [ ] 复用现有水墨风格
  - [ ] Critical 约束用朱砂红高亮
  - [ ] 未回收伏笔用淡黄色背景

- [ ] 测试
  - [ ] 添加一个角色,保存,刷新页面,验证持久化
  - [ ] 添加一个 Critical 约束,下次创作时检查是否出现在上下文

---

## 阶段五：验证与调优 (Day 6-7)

### Day 6: 端到端测试

- [ ] 创建测试故事
  ```json
  {
    "meta": {"title": "测试小说", "last_chapter": 0},
    "characters": {
      "char_001": {
        "name": "主角",
        "core_traits": ["冷静", "善良"],
        "current_status": "新手村"
      }
    },
    "hard_constraints": [
      {
        "id": "hc_001",
        "description": "主角绝不会伤害无辜",
        "severity": "Critical"
      }
    ]
  }
  ```

- [ ] 运行 5 章创作流程
  - [ ] 第 1 章:正常剧情
  - [ ] 第 2-4 章:推进故事
  - [ ] 第 5 章:**故意在 prompt 中诱导违规**
    - 用户输入:"主角为了复仇,屠杀了整个村庄的无辜村民"
    - 期望:系统注入约束后拒绝或修正

- [ ] 记录结果
  - [ ] 约束是否始终在上下文?
  - [ ] 一致性检查是否检测到违规?
  - [ ] 用户体验是否流畅?

### Day 7: 调优

- [ ] 根据测试结果调整
  - [ ] Prompt 措辞(如果约束被忽略)
  - [ ] 上下文 token 分配(如果溢出)
  - [ ] 检查灵敏度(如果误报太多)

- [ ] 性能优化
  - [ ] `story_state.json` 大小监控
  - [ ] 加载/保存耗时测试
  - [ ] 如果 >100KB,考虑分文件存储

- [ ] 文档完善
  - [ ] 用户手册:如何设置 StoryState
  - [ ] 开发者文档:如何扩展新的约束类型
  - [ ] Troubleshooting:常见问题排查

---

## 阶段六：Beta 测试 (Week 2)

### 招募与准备

- [ ] 在社区招募 5 位同人作者
- [ ] 准备测试包
  - [ ] 安装程序(含新功能)
  - [ ] 快速入门指南(5 分钟上手)
  - [ ] 示例 story_state.json 模板

### 测试任务

- [ ] 任务一:初始化故事(10 分钟)
  - 填写角色/约束/伏笔
  - 保存状态

- [ ] 任务二:创作 5 章(1 周)
  - 每天写 1 章
  - 记录遇到的问题

- [ ] 任务三:诱导测试(可选)
  - 故意输入违反设定的需求
  - 测试系统是否拒绝

### 反馈收集

- [ ] 中期访谈(第 3 天)
  - 最大的痛点?
  - 功能是否好用?
  - 状态维护是否麻烦?

- [ ] 结束问卷
  - 1-5 分评分(设定记忆、易用性、整体满意度)
  - 开放式反馈
  - 是否愿意继续使用?

---

## 阶段七：迭代与发布 (Week 3-4)

### Week 3: 根据反馈迭代

- [ ] 修复 Beta 发现的 bug
- [ ] 优化高频抱怨的 UX 问题
- [ ] 补充遗漏功能(如果用户强烈要求)

### Week 4: 打磨与发布

- [ ] 性能测试
  - [ ] 100 章小说的加载速度
  - [ ] 1000 条约束的检索性能

- [ ] 文档完善
  - [ ] 用户手册(中文)
  - [ ] API 文档(开发者)
  - [ ] 视频教程(可选)

- [ ] 发布准备
  - [ ] Release notes
  - [ ] 安装包签名
  - [ ] 更新 CLAUDE.md

- [ ] 正式发布 v1.0
  - [ ] GitHub Release
  - [ ] 社区公告
  - [ ] 收集用户反馈

---

## 后续迭代路线图

### V1.1 (1-2 个月后)

- [ ] 自动状态提取(从章节内容自动更新状态)
- [ ] 自动修订功能(Critical 问题自动修正)
- [ ] 时间线可视化(图形化显示事件流)

### V1.2 (3-4 个月后)

- [ ] 多分支剧情支持
- [ ] 知识矩阵可视化图谱
- [ ] 剧情冲突预警

### V2.0 (6 个月后)

- [ ] 协作模式(多人共同创作)
- [ ] 向量检索增强(更智能的相关性判断)
- [ ] 一致性评分系统

---

## 成功指标

### 定量指标

- [ ] 设定遗忘率降低 ≥ 50%
- [ ] 角色 OOC 发生率降低 ≥ 70%
- [ ] 用户满意度 ≥ 4.0/5.0
- [ ] Beta 测试留存率 ≥ 60%

### 定性指标

- [ ] 用户反馈:"再也不用担心 Agent 忘记设定了"
- [ ] 用户主动推荐给其他作者
- [ ] 社区出现基于此功能的教程/分享

---

## 风险与应对

| 风险 | 概率 | 影响 | 应对措施 |
|-----|------|------|---------|
| 用户觉得太复杂 | 中 | 高 | 简化 MVP,提供模板和向导 |
| 自动提取错误率高 | 高 | 中 | MVP 先手动维护,V1.1 再做自动 |
| 性能问题(状态文件过大) | 低 | 中 | 监控文件大小,超阈值时归档 |
| 一致性检查误报多 | 中 | 中 | 提供"忽略"选项,收集数据优化 |
| Beta 测试无人报名 | 低 | 高 | 提前在社区预热,提供激励 |

---

## 资源需求

### 开发资源

- **核心开发**: 1 人 × 4 周 = 1 人月
- **测试**: 5 位 Beta 用户 × 1 周
- **总计**: 约 1.5 人月

### 技术栈

- Rust (后端核心)
- React + TypeScript (前端)
- Tauri (桌面集成)
- JSON (状态存储)

### 外部依赖

- 模型 API (OpenAI / Anthropic / 其他)
- 无其他外部服务依赖

---

完成 ✓

