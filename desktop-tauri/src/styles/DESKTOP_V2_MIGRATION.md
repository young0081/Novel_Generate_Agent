# 桌面端 v2 重构 —— 已完成 + 待续

## 重构成果

### 核心改进：11 屏扁平化 → 3 层工作流

**旧架构问题**：11 个平级屏幕挤在左侧边栏（策划/探讨/创作/会话/章节/人物/伏笔/设定/协作/快照/工具/供应商），认知负担重、视觉混乱、所有屏用同样的卡片网格。

**新架构（书桌工作流）**：
- **顶部横向标签**：策划 | 创作 | 修订（3 个主工作模式）+ 右侧设置按钮
- **删除整个左侧边栏**
- **设置弹层**：供应商配置 + 工具目录，从底部滑出的全幅 sheet

### 视觉语言

- 保持水墨色系（宣纸 #ece3d2/墨 #25201c/朱砂 #b73225），但**收敛令牌**
- 编辑式排版：kicker（朱砂小标）+ serif 大标题 + 留白
- 横向标签激活态：底部 3px 朱砂下划线
- 面板分隔：1px 朱砂虚线
- 全程 transform/opacity 动效，尊重 reduced-motion

## 文件结构

### 新增
```
src/
├── App.tsx                        # 重写：3 模式路由 + 设置弹层
├── components/
│   ├── work/WorkTabs.tsx          # 顶部横向标签组
│   ├── drawer/Drawer.tsx          # 右侧滑入抽屉（通用）
│   └── SettingsModal.tsx          # 设置弹层（供应商+工具，含 legacy-scope）
├── screens/
│   ├── PlanningWork.tsx           # 策划：构思 + 生成设定（4 个 agent 动作）
│   ├── StudioWork.tsx             # 创作：agent 运行 + 实时馈给 + 成稿
│   └── RevisionWork.tsx           # 修订：章节列表 + 编辑器
└── styles/
    ├── theme.css                  # 重写：精简水墨令牌系统
    ├── app.css                    # 重写：shell/标签/面板/抽屉/按钮 + agent 组件样式
    ├── work-screens.css           # 三个工作屏 + agent 组件 + 设置弹层样式
    └── legacy.css                 # v1 样式（作用域化到 .legacy-scope，供设置弹层复用）
```

### 复用（未改）
- `components/agent/*`：WorkStatus/WorkflowSteps/AgentFeed/ReasoningBlock/ToolCallCard/Conversation
- `components/Toast.tsx`、`TitleBar.tsx`、`ConfirmModal.tsx`、`Spinner.tsx`、`icons.tsx`
- `lib/*`：core/studio/agentRun/providers/memory/sessions/window 全部 API 契约保留
- `screens/ProvidersScreen.tsx`、`ToolsScreen.tsx`：在设置弹层里原样复用（legacy-scope 隔离样式）

### 旧屏幕（保留磁盘，已脱离路由）
以下旧屏不再被 App.tsx 引用，但代码保留，功能已整合或待迁移：
- `PlanningScreen.tsx` → 已整合进 `PlanningWork.tsx`
- `StudioScreen.tsx` → 已整合进 `StudioWork.tsx`
- `ChaptersScreen.tsx` → 已整合进 `RevisionWork.tsx`
- `ChatScreen.tsx`（探讨）→ **待迁移**：可作为策划标签下的一个面板，或独立抽屉
- `SessionsScreen.tsx`（会话历史）→ **待迁移**：建议做成右侧抽屉
- `MemoryScreen.tsx`（人物/伏笔/设定库）→ **待迁移**：建议做成右侧抽屉（只读查询）
- `CollabScreen.tsx`（协作）+ `CheckpointsScreen.tsx`（快照）→ **待迁移**：合并为协作抽屉

## 已接入的真实功能

| 工作屏 | 真实功能 | 后端调用 |
|--------|---------|---------|
| 策划 | 构思输入 + 4 个生成动作（世界观/人物/大纲/伏笔），实时 agent 馈给 | `runGoalLive` + `getProviders` |
| 创作 | 创作目标 + agent loop + 工作流程/状态/推理/工具卡 + 成稿 + 全程记录 | `runGoalLive` |
| 修订 | book/ 与根目录浏览、读写、新建、删除章节 | `list_dir`/`read_file`/`write_file`/`delete_file` |
| 设置 | 供应商增删改/测试连接/快速填充、工具目录 | `ProvidersScreen`/`ToolsScreen`（原样复用） |

## 待迁移项的建议方案

### 1. 会话历史抽屉（SessionsScreen）
在 WorkTabs 右侧加一个「会话」按钮，点击用 `Drawer` 组件从右滑入，列出 `sessions_list` 的存档，点击「继续创作」跳到创作标签并 resume。

### 2. 记忆库抽屉（MemoryScreen — 人物/伏笔/设定）
同样用 `Drawer`，内部用标签切换 character/foreshadow/setting，卡片网格展示 `memory_recall` 结果，只读 + 删除。

### 3. 探讨面板（ChatScreen）
作为策划标签下的第三个面板（构思 → 探讨 → 生成设定），复用 `Conversation` 组件 + `chatStream`。

### 4. 协作抽屉（CollabScreen + CheckpointsScreen）
合并为一个抽屉，标签切换「版本历史」（commits/diff/branch）和「快照」（checkpoints）。

### 迁移模式
所有待迁移屏都已有完整的功能逻辑，迁移只需：
1. 把屏内容包进 `Drawer` 或新面板
2. 用新的 `.drawer__*` / `.panel` / `.card` 类替换旧类（或临时套 `.legacy-scope`）
3. 在 App.tsx 加 drawer 开关状态 + WorkTabs 触发按钮

## 验证

- ✅ `npm run build`：tsc 零错误，vite 构建通过（CSS 124KB 含 legacy，JS 276KB）
- ✅ `tauri build --no-bundle`：桌面端 release 编译通过（desktop-tauri.exe ≈14MB）
- ✅ dev server 冒烟：HTTP 200，root 挂载正常
- ✅ 三个主工作流接入真实后端 API
