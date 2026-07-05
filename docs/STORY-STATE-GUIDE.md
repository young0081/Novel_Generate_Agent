# Story State 系统使用指南

## 概述

Story State 系统是为解决长篇连续创作中"设定遗忘"和"逻辑不一致"问题而设计的状态管理系统。它通过结构化记录和自动注入的方式，确保 AI 在每次生成时都能看到完整且一致的剧情状态。

## 核心功能

### 1. 结构化状态管理

系统维护以下状态：

- **角色状态** (`CharacterState`)
  - 核心特征（用于 OOC 检测）
  - 当前处境
  - 目标与动机

- **时间线** (`Timeline`)
  - 当前章节
  - 已发生的关键事件

- **知识矩阵** (`KnowledgeMatrix`)
  - 记录每个角色知道什么信息
  - 防止角色"凭空得知"不该知道的事

- **伏笔追踪** (`ForeshadowTracker`)
  - 已埋下的伏笔
  - 状态：Planted → Hinted → Resolved

- **硬约束** (`Constraint`)
  - 5 个严重等级：Info < Low < Medium < High < Critical
  - 按优先级排序，Critical 永远在最前
  - 例如："主角绝不背叛朋友"（Critical）

- **软偏好** (`Preference`)
  - 非强制的创作风格偏好
  - 例如："战斗要有策略性"

### 2. 自动状态注入

当用户在"创作"界面发起创作时，系统会：

1. 检查 `workspace/story_state.json` 是否存在
2. 如果存在，加载状态并准备上下文包
3. 将状态渲染成清晰的 Prompt
4. 作为系统消息注入到会话中
5. AI 在看到创作目标前，先看到完整状态

注入的 Prompt 格式示例：

```
# 当前剧情状态同步 (第 5 章)

## 核心角色当前状态
- **林惊羽**:
  - 核心特征: 冷静, 重情义, 不轻易承诺
  - 当前处境: 练气九层,准备突破筑基
  - 目标: 找到杀师仇人; 保护师妹

## ⚠️ 必须遵守的硬约束
- **[Critical]** 林惊羽绝不会背叛朋友
- **[High]** 筑基期以下无法御剑飞行(世界规则)

## 🌱 未回收伏笔
- 师傅临终时眼神看向北方 (埋于第1章)

---
**重要**: 续写时必须严格符合以上状态。
```

### 3. Tauri 命令接口

桌面端提供三个命令：

```typescript
// 加载状态
const state = await invoke('story_state_load');

// 保存状态
await invoke('story_state_save', { state: newState });

// 准备上下文预览
const ctx = await invoke('story_state_prepare_context', { 
  chapterNum: 5 
});
```

## 使用流程

### 初始化项目

1. 在"策划"界面完成世界观设定后，手动创建 `story_state.json`：

```json
{
  "meta": {
    "title": "作品标题",
    "genre": "类型",
    "last_chapter": 0
  },
  "world": {
    "rules": [
      {
        "id": "wr_001",
        "description": "世界规则1"
      }
    ]
  },
  "characters": {},
  "timeline": {
    "current_chapter": 0,
    "events": []
  },
  "knowledge_matrix": {
    "entries": {}
  },
  "foreshadows": [],
  "hard_constraints": [],
  "soft_preferences": [],
  "current_chapter_goal": null
}
```

2. 或使用 Rust 辅助函数创建：

```rust
let mut mgr = StoryStateManager::open("workspace/story_state.json")?;
mgr.state.meta.title = "修仙逆袭录".to_string();
mgr.state.meta.genre = "修仙/热血".to_string();

// 添加角色
mgr.add_character(CharacterState {
    id: "char_001".to_string(),
    name: "林惊羽".to_string(),
    core_traits: vec!["冷静".to_string(), "重情义".to_string()],
    current_status: "练气九层".to_string(),
    goals: vec!["找到杀师仇人".to_string()],
});

// 添加硬约束
mgr.add_constraint(
    "hc_001".to_string(),
    "林惊羽绝不会背叛朋友".to_string(),
    Severity::Critical,
);

mgr.save()?;
```

### 创作章节

1. 用户在"创作"界面输入目标：
   ```
   写第一章：师傅遇害，主角立誓复仇
   ```

2. 系统自动：
   - 加载 `story_state.json`
   - 注入状态到上下文
   - AI 生成章节内容
   - 严格遵守角色特征和约束

3. 章节完成后，手动或自动更新状态：

```rust
// 记录事件
mgr.add_timeline_event(1, "师傅被害，主角立誓复仇".to_string());

// 埋下伏笔
mgr.plant_foreshadow(
    "fh_001".to_string(),
    "师傅临终时眼神看向北方".to_string(),
    1,
);

// 更新角色状态
mgr.update_character_status(
    "char_001",
    "练气九层，悲痛欲绝但强忍怒火".to_string(),
);

// 推进到下一章
mgr.advance_chapter();
mgr.save()?;
```

### 持续创作

每次创作新章节时：

1. 系统自动加载最新状态
2. 将 `last_chapter + 1` 作为当前章节
3. 过滤出高优先级约束（High 以上）
4. 筛选未回收的伏笔
5. 注入到 AI 上下文

AI 将在生成时：
- 看到角色的核心特征
- 看到最近 3 个事件
- 看到必须遵守的约束
- 记得未回收的伏笔

## 一致性检查（未来扩展）

`ConsistencyGuard` 模块提供后生成检查能力（当前为占位实现）：

```rust
let guard = ConsistencyGuard::new();
let report = guard.check_basic(&chapter_content, &story_state)?;

if report.has_critical_issues() {
    // 显示问题并要求修订
    for issue in report.issues {
        println!("{:?}: {}", issue.severity, issue.description);
    }
}
```

未来将集成模型进行自动检查：
- 角色 OOC 检测
- 知识泄漏检测
- 时间线矛盾检测
- 约束违反检测

## 前端集成（待实现）

建议在桌面端添加"状态管理"界面：

```
┌─────────────────────────────────────┐
│ 📋 剧情状态管理                      │
├─────────────────────────────────────┤
│                                     │
│ 角色 [2]        | 添加角色           │
│  - 林惊羽       | 编辑 删除          │
│  - 苏清雪       |                   │
│                                     │
│ 硬约束 [3]      | 添加约束           │
│  🔴 Critical: 林惊羽绝不背叛朋友      │
│  🟠 High: 筑基期以下无法御剑飞行      │
│                                     │
│ 伏笔 [2]        | 添加伏笔           │
│  🌱 师傅临终时眼神看向北方 (第1章)    │
│  🌱 神秘玉佩在月圆之夜会发光 (第1章)  │
│                                     │
│ 时间线 [第1章]   | 查看完整时间线     │
│                                     │
│ [保存状态] [预览注入内容]            │
└─────────────────────────────────────┘
```

## 测试

运行完整测试套件：

```bash
cd core
cargo test -p na-story
```

当前测试覆盖：
- ✅ 16 个单元测试全部通过
- 状态序列化/反序列化
- 知识矩阵查询
- 约束优先级排序
- 伏笔过滤
- Prompt 生成
- 状态更新辅助函数

运行演示：

```bash
cd core/crates/na-story
cargo run --example demo
```

## 故障排查

### 问题：AI 仍然忘记设定

**检查项**：
1. `story_state.json` 是否存在于 workspace？
2. 状态是否正确保存？
3. 查看会话的第一条系统消息，确认状态已注入
4. 约束是否标记为 High/Critical？

### 问题：角色行为 OOC

**解决方案**：
1. 检查 `core_traits` 是否明确
2. 提高相关约束的 `severity`
3. 在"策划"时让 AI 生成详细的角色卡

### 问题：状态文件过大

**优化**：
1. 定期归档旧事件（只保留最近 10 条）
2. 已解决的伏笔可以移除
3. 考虑拆分为多个文件（未来版本）

## 性能影响

- 文件 I/O：每次创作加载一次（~1ms）
- Prompt 增量：约 300-800 tokens（取决于状态复杂度）
- 上下文窗口：现代模型足够容纳（Claude 200k+）

## 下一步优化方向

1. **自动状态提取**：创作完成后，让 AI 自动提取需要更新的状态
2. **GUI 状态编辑器**：可视化编辑角色、约束、伏笔
3. **一致性自动检查**：后生成调用模型检查矛盾
4. **状态分层**：区分"全局设定"和"当前状态"
5. **状态版本控制**：配合 checkpoint 做状态快照

---

**版本**: Day 2 (2026/06/24)  
**状态**: ✅ 核心功能已实现并集成
