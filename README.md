# Novel Generate Agent（AI 驱动的小说创作平台）

<div align="center">

![License](https://img.shields.io/badge/license-MIT-blue.svg)
![Rust](https://img.shields.io/badge/rust-1.80%2B-orange.svg)
![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20Android%20%7C%20iOS%20%7C%20Web-lightgrey.svg)

**一个完整的 AI 创作平台 + 一套类 Claude Code 的 AI Agent 运行时**

[功能特性](#核心特性) • [快速开始](#快速开始) • [架构设计](#技术架构) • [贡献指南](#贡献)

</div>

---

## 项目简介

这是一个面向生产级约束持续完善的 **AI 创作平台**，核心是用 Rust 构建的 **AI Agent 运行时**（类似 Claude Code 的引擎）。它不仅能让 AI 辅助创作小说，也展示了如何构建一个安全、可靠、可扩展的 AI Agent 系统。

### 为什么做这个项目？

- **技术挑战**：AI Agent 需要工具调用、沙箱隔离、上下文管理、流式输出、状态恢复——这是一个完整的系统工程问题
- **实际需求**：长篇创作面临"AI 会忘设定"的痛点，需要 Story State 系统来保持一致性
- **架构实践**：展示如何用 Rust 构建高性能运行时，用 Tauri 打造原生跨平台 GUI

### 核心特性

#### 🦀 Rust Agent 运行时（核心层）
- **工具系统**：23 个基础工具（文件/搜索/网络/版本控制/记忆管理）；完整运行时另提供 3 个技能/子代理工具
- **沙箱隔离**：路径监狱 + 能力白名单 + 资源预算，防止越界访问
- **上下文管理**：自动压缩 + 长期记忆 RAG（BM25 检索）+ checkpoint/回滚机制
- **防护机制**：Prompt 注入防护 + 防死循环（步数/预算/重复检测）+ 取消令牌
- **Story State 系统**：角色状态、知识矩阵、伏笔和硬约束的结构化存储与上下文注入

#### 🎨 水墨国风桌面端（Tauri）
- **原生架构**：Rust 核心直接编译为原生后端（无独立进程 / 无 Node 服务 / 用系统 WebView2）
- **视觉设计**：宣纸底 + 水墨灰 + 朱砂红 + 宋体 + 篆刻印章 + 书法笔触
- **实时创作**：流式输出 AI 思考过程 + 工具调用可视化 + 运笔动画
- **IDE 编辑器**：Cursor 风格三栏工作台（文件树 + 编辑器 + AI 助手），对话/运笔双模式，AI 直接改稿
- **版本管理**：文学 git（分支/diff/回滚）+ 多供应商模型管理 + 会话续写
- **批量管理**：记忆、会话、作品、知识库条目、快照、书稿文件与供应商均支持勾选、全选、顺序批量删除，并显示进度与部分失败结果

#### 📱 跨平台支持
- **Windows**：Tauri 桌面端 + 自定义无边框单文件安装器
- **Android / iOS**：Flutter 独立移动端（本地存储，直连用户配置的模型 API）
- **Web**：Next.js 创作工作台（通过 JSON-RPC 调用核心层）

---

## 技术架构

```
Tauri 桌面端 ── 原生内嵌 ─────┐
                              ├── Rust Agent 运行时
Next.js Web ── JSON-RPC/stdio ┘   (runtime / tools / sandbox / memory / story)

Flutter 移动端 ── HTTPS ── 模型供应商 API
              └── 设备本地章节 / 记忆 / 快照
```

### Cargo Workspace 结构

- **na-common**：共享类型、错误归一化、取消令牌
- **na-sandbox**：沙箱实现（路径监狱、能力白名单、资源预算）
- **na-tools**：23 个基础工具 + 工具注册表 + 参数校验 + 输出处理管线
- **na-memory**：checkpoint/回滚、审计日志、BM25 长期记忆 RAG
- **na-story**：Story State 系统（角色状态、知识矩阵、伏笔追踪、硬约束）
- **na-runtime**：会话/上下文管理、agent 自循环、模型编排、ReAct 协议、Prompt 注入防护
- **na-host**：JSON-RPC 主机（GUI 后端进程） + 验证用二进制

**质量门禁**：workspace 单元测试、集成测试和 doctest；提交前运行 `cargo test --workspace`、`cargo clippy --workspace --all-targets -- -D warnings` 与 `cargo fmt --all -- --check`。

---

## 快速开始

### 前置要求

- **Rust** 1.80+（[安装指南](https://rustup.rs/)）
- **Node.js** 20.19+（桌面端/Web 端需要）
- **Flutter** 3.44+（仅移动端需要）

### 核心层能力概览

把它想象成一个「会用工具、有记忆、能自我约束的 AI 写作助手的大脑」：

- **会用工具**：读写文件、搜索、抓网页、注册外部 MCP 工具、给小说做版本管理；生产默认不开放任意 shell。
- **有边界**：所有操作都被关在「工作区沙箱」里，跑不出去；危险命令（比如删硬盘）会被直接拦下。
- **有记忆**：人物、设定、伏笔会存进长期记忆库，需要时用「搜索 + 摘要」的方式回忆（不会把全部内容硬塞回 AI，省钱又准）。
- **能反悔**：随时给手稿拍快照（checkpoint），写崩了一键回滚——而且回滚只还原稿子，长期记忆和操作日志不受影响。
- **不会卡死**：AI 自循环干活时有多重「刹车」（步数上限、超时、重复动作检测、无进展检测），绝不会无限空转。
- **防忽悠**：网页/外部工具返回的内容会被标记为「不可信」并做净化，防止「忽略以上指令」这类提示词注入攻击。
- **能随时喊停**：任何时候都能取消/中断正在进行的工作。

---

### 1. 运行核心层演示（推荐）

核心层会自动模拟 AI 创作流程：写章节 → 存记忆 → 做快照 → 模拟写崩 → 一键恢复。

```bash
cd core
cargo run -p na-host --bin demo
```

### 2. 运行桌面端（完整体验）

```bash
# 开发模式（会自动编译，弹出原生窗口）
cd desktop-tauri
npm install
npm run tauri dev
```

**首次使用流程**：
1. 打开应用 → 左下角「**设置**」→「**供应商**」→ 新增你的 AI 服务商（OpenAI / DeepSeek / Claude 等）
2. 填入 **API Key** + 选择模型 → 测试连接 → 设为当前
3. 左侧「**策划**」→ 输入作品构思 → 与 AI 探讨 → 生成世界观/人物/大纲
4. 左侧「**创作**」→ 输入创作目标 + 章节标题 → 实时看 AI 运笔 → 产出章节
5. 左侧「**编辑**」→ Cursor 风格 IDE 精修章节：左侧文件树 + 中间编辑器 + 右侧 AI 助手（对话续写 / 运笔改稿）
6. 左侧「**修订**」→ 改稿；「**协作**」→ 做版本管理（提交/分支/对比）

列表中的「批量管理」只作用于当前可见条目；删除前会二次确认，批处理按顺序执行，单项失败不会阻塞后续项目。移动端章节、记忆、快照与历史会话也提供相同的批量删除体验。

> 你的 API Key 只保存在本机（`%APPDATA%\com.novelgenerateagent.desktop\providers.json`）。

### 3. 运行 Web 端（浏览器版）

```bash
# 先编译核心后端
cd core
cargo build -p na-host --release

# 启动前端
cd ../frontend
npm install
npm run dev
```

打开 http://localhost:3000

> Web 端默认只监听 `127.0.0.1`，因为 RPC 可以操作本地工作区。不要在没有鉴权和 TLS 的情况下将它公开部署到网络。

---

## 核心能力详解

### 🛠️ 工具系统

基础引擎注册 23 个工具；完整运行时可再注册 3 个技能/子代理工具：

| 类别 | 工具 |
|------|------|
| **文件与搜索** | read_file, write_file, edit_file, delete_file, list_dir, search |
| **记忆管理** | memory_save, memory_recall, memory_list, memory_classify, memory_archive, memory_delete |
| **检查点** | checkpoint_create, checkpoint_list, checkpoint_restore, checkpoint_delete |
| **文学版本控制** | vcs_commit, vcs_log, vcs_diff, vcs_restore, vcs_branch |
| **执行与网络** | shell（生产默认禁用）, web_fetch |
| **完整运行时扩展** | skill_list, skill_load, spawn_subagent |

每个工具都经过：**参数校验（JSON Schema）→ 权限检查 → 沙箱执行 → 输出处理管线 → 审计落盘**。

### 🔒 安全机制

#### 沙箱隔离
```rust
// 所有文件操作都被限制在工作区内
let sandbox = Sandbox::new(workspace_path)
    .with_capabilities(Capabilities::READ | Capabilities::WRITE)
    .with_max_file_size(10 * 1024 * 1024)  // 10MB
    .with_timeout(Duration::from_secs(30));

// 尝试访问工作区外的路径会被拒绝
sandbox.validate_path("../../etc/passwd")?;  // Error: 越界访问
```

#### Prompt 注入防护
```rust
// 工具返回的内容会被标记为不可信并净化
let output = tool.execute()?;
let sanitized = output_pipeline
    .detect_instruction_patterns()  // 检测"忽略以上指令"等
    .strip_ansi_codes()
    .redact_secrets()               // 脱敏敏感信息
    .truncate(max_bytes)
    .process(output)?;
```

### 🧠 Story State 系统（解决"忘设定"问题）

长篇创作中 AI 容易忘记之前的设定。Story State 系统提供结构化状态和可选的上下文注入：

```rust
pub struct StoryState {
    pub characters: HashMap<String, CharacterState>,  // 角色状态
    pub knowledge_matrix: KnowledgeMatrix,            // 知识矩阵（谁知道什么）
    pub foreshadows: Vec<ForeshadowTracker>,         // 伏笔追踪
    pub hard_constraints: Vec<Constraint>,           // 硬约束（5 级严重性）
    pub timeline: Timeline,                          // 时间线
    pub world: WorldState,                           // 世界状态
}
```

**当前工作流程**：
1. 创作前：`prepare_context(current_chapter)` 提取当前章节需要的状态
2. 当工作区已有 `story_state.json` 时，桌面创作流程会把状态渲染成 Prompt 并注入会话
3. 写作流程会在生成前注入本章状态与大纲节点，生成后推进章节游标并回写时间线；状态文件也可通过核心 API 管理

**示例注入 Prompt**：
```markdown
# 当前剧情状态同步 (第 5 章)

## 核心角色当前状态
- **林惊羽**: 冷静/重情义；练气九层准备突破筑基；目标：找到杀师仇人

## ⚠️ 必须遵守的硬约束
- [Critical] 林惊羽绝不会背叛朋友
- [High] 筑基期以下无法御剑飞行（世界规则）

## 🌱 未回收伏笔
- 师傅临终时眼神看向北方（埋于第1章）
```

详见 [Story State 使用指南](./docs/STORY-STATE-GUIDE.md)。

### ✍️ IDE 编辑器（Cursor 风格创作工作台）

桌面端内置一套三栏 IDE，把「写稿」和「AI 协作」放进同一个界面，不用在多个页面间来回切换：

```
┌──────────┬────────────────────────┬──────────────┐
│  文件树  │       编辑器           │   AI 助手    │
│          │  (CodeMirror 6         │              │
│ book/    │   水墨主题 + 自动保存) │ ┌──────────┐ │
│  第一章  │                        │ │对话│运笔│ │
│  第二章  │  # 第一章               │ └──────────┘ │
│  ...     │  林惊羽握紧手中的剑…    │ 模型: DeepSeek│
│          │                        │ ▍流式回复…   │
│ [右键]   │                        │ [↓ 插入编辑器]│
└──────────┴────────────────────────┴──────────────┘
  行 12, 列 8 · 1024 字 · ✓ 已保存
```

**核心能力**：

- **双模式 AI 助手**
  - **对话模式**：流式聊天问答，AI 回复可一键「插入到编辑器」光标处
  - **运笔模式**：跑完整 Agent loop，AI **直接调用工具修改文件**，实时可视化推理过程与工具调用；改完编辑器自动从磁盘刷新（保留光标）
- **跨章节上下文**：在单独章节中问稿或改稿时，自动按章节顺序读取同目录的前文章节（有数量与字数上限），并将其作为只读参考，避免人物与情节断档
- **模型即时切换**：编辑器内嵌 ModelSelector，无需跳转设置页即可切换供应商/模型
- **文件管理**：文件树右键菜单（重命名 / 删除）、多标签页编辑
- **编辑体验**：CodeMirror 6 编辑器 + Ctrl+F 内建搜索 + 底部状态栏（行列 / 字数 / 保存状态）+ 800ms 防抖自动保存
- **可调布局**：三栏宽度可拖拽调节，偏好持久化到本地

---

## 打包分发

### Windows 安装包

```bash
# 1. 构建桌面端
cd desktop-tauri
npm run tauri build -- --bundles nsis

# 2. 构建自定义安装器（单文件 exe）
cp "src-tauri/target/release/desktop-tauri.exe" "../installer/src-tauri/payload/NovelGenerateAgent.exe"
cd ../installer
npm install
npm run tauri build -- --no-bundle
```

产物：`installer/src-tauri/target/release/installer.exe`（体积取决于当前 payload，双击即装，支持检测已有安装并原地更新）。

各版本 Windows 安装程序统一归档在 [`releases/`](releases/) 下，按版本号分目录保存；当前版本为 `releases/v0.3.2/`。

### Android APK

```bash
cd mobile
flutter build apk --release
```

产物：`build/app/outputs/flutter-apk/app-release.apk`（约 52MB）。

---

## 项目目录结构

```
Novel_Generate_Agent/
├── core/                     # Rust 核心层（cargo workspace）
│   ├── Cargo.toml            # workspace 根
│   ├── crates/
│   │   ├── na-common/        # 共享类型、错误、取消令牌
│   │   ├── na-sandbox/       # 沙箱实现
│   │   ├── na-tools/         # 工具注册表 + 23 个基础工具
│   │   ├── na-memory/        # checkpoint/回滚 + 审计日志 + RAG
│   │   ├── na-story/         # Story State 系统
│   │   ├── na-runtime/       # Agent 运行时 + 模型编排
│   │   └── na-host/          # JSON-RPC 主机 + 验证二进制
│   └── tests/                # 跨 crate 集成测试
├── desktop-tauri/            # Tauri 桌面端（水墨国风，Rust 核心原生内嵌）
├── installer/                # 自定义无边框安装器（单文件 exe）
├── releases/                 # 各版本 Windows 安装程序归档
├── frontend/                 # Next.js Web 端
├── mobile/                   # Flutter 移动端（Android / iOS）
├── brand/                    # 品牌资源（图标/logo）
└── docs/                     # 技术文档
```

---

## 技术栈

| 层 | 技术 | 说明 |
|---|---|---|
| **核心运行时** | Rust (cargo workspace) | Agent 运行时 + 工具执行层 + 沙箱隔离 |
| **桌面端** | Tauri + React + Vite | Rust 核心原生内嵌，用系统 WebView2 |
| **Web 端** | Next.js 16 + TypeScript | 通过本地 Node 桥（JSON-RPC/stdio）调用核心 |
| **移动端** | Flutter 3.44 + Dart 3.12 | 直连 OpenAI 兼容 / Anthropic API，数据保存在设备本地 |
| **模型接入** | OpenAI 兼容 / Anthropic / Gemini | 桌面核心支持原生工具调用、ReAct 兼容和流式输出（SSE） |
| **通信协议** | JSON-RPC 2.0 | Web 端通过本地 Node 代理连接 Rust stdio host |

---

## 贡献

欢迎贡献代码、报告问题或提出建议！

### 开发环境搭建

```bash
# 1. 克隆仓库
git clone https://github.com/young0081/Novel_Generate_Agent.git
cd Novel_Generate_Agent

# 2. 编译核心层
cd core
cargo build
cargo test --workspace

# 3. 运行桌面端
cd ../desktop-tauri
npm install
npm run tauri dev
```

### 代码规范

- Rust 代码：遵循 `rustfmt` + `clippy` 规则
- 前端代码：遵循 ESLint + TypeScript 严格模式
- 提交信息：使用清晰的中文或英文描述

### 架构文档

- [Story State 使用指南](./docs/STORY-STATE-GUIDE.md) - 剧情状态管理系统

---

## 许可证

本项目采用 [MIT License](./LICENSE) 开源。

---

## 致谢

- Rust 生态：[tokio](https://tokio.rs/), [serde](https://serde.rs/), [reqwest](https://docs.rs/reqwest/)
- Tauri 团队：提供出色的跨平台桌面框架
- Anthropic & OpenAI：AI 能力支持

---

## 联系方式

- GitHub Issues: [提交问题](https://github.com/young0081/Novel_Generate_Agent/issues)
- 项目作者: [@young0081](https://github.com/young0081)

---

<div align="center">

**如果这个项目对你有帮助，欢迎 Star ⭐**

Made with ❤️ and Rust 🦀

</div>
