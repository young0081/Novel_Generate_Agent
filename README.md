# Novel Generate Team（AI 驱动的小说创作平台）

<div align="center">

![License](https://img.shields.io/badge/license-MIT-blue.svg)
![Rust](https://img.shields.io/badge/rust-1.75%2B-orange.svg)
![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20Android-lightgrey.svg)

**一个完整的 AI 创作平台 + 一套类 Claude Code 的 AI Agent 运行时**

[功能特性](#核心特性) • [快速开始](#快速开始) • [架构设计](#技术架构) • [贡献指南](#贡献)

</div>

---

## 项目简介

这是一个**生产级 AI 创作平台**，核心是用 Rust 从零构建的 **AI Agent 运行时**（类似 Claude Code 的引擎）。它不仅能让 AI 辅助创作小说，更重要的是展示了如何构建一个**安全、可靠、可扩展的 AI Agent 系统**。

### 为什么做这个项目？

- **技术挑战**：AI Agent 需要工具调用、沙箱隔离、上下文管理、流式输出、状态恢复——这是一个完整的系统工程问题
- **实际需求**：长篇创作面临"AI 会忘设定"的痛点，需要 Story State 系统来保持一致性
- **架构实践**：展示如何用 Rust 构建高性能运行时，用 Tauri 打造原生跨平台 GUI

### 核心特性

#### 🦀 Rust Agent 运行时（核心层）
- **工具系统**：26 个内建工具（文件/搜索/网络/版本控制/记忆管理），完整生命周期管理
- **沙箱隔离**：路径监狱 + 能力白名单 + 资源预算，防止越界访问
- **上下文管理**：自动压缩 + 长期记忆 RAG（BM25 检索）+ checkpoint/回滚机制
- **防护机制**：Prompt 注入防护 + 防死循环（步数/预算/重复检测）+ 取消令牌
- **Story State 系统**：角色状态追踪 + 知识矩阵 + 伏笔追踪 + 硬约束管理，解决"忘设定"问题

#### 🎨 水墨国风桌面端（Tauri）
- **原生架构**：Rust 核心直接编译为原生后端（无独立进程 / 无 Node 服务 / 用系统 WebView2）
- **视觉设计**：宣纸底 + 水墨灰 + 朱砂红 + 宋体 + 篆刻印章 + 书法笔触
- **实时创作**：流式输出 AI 思考过程 + 工具调用可视化 + 运笔动画
- **版本管理**：文学 git（分支/diff/回滚）+ 多供应商模型管理 + 会话续写

#### 📱 跨平台支持
- **Windows**：Tauri 桌面端 + 自定义无边框安装器（单文件 exe，约 23MB）
- **Android**：Flutter 移动端（复用同一 Rust 核心）
- **Web**：Next.js 创作工作台（通过 JSON-RPC 调用核心层）

---

## 技术架构

```
┌─────────────────────────────────────────────────────────────┐
│                        GUI 层                                │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐      │
│  │ Tauri 桌面端 │  │ Flutter 移动 │  │  Next.js Web │      │
│  └──────┬───────┘  └──────┬───────┘  └──────┬───────┘      │
└─────────┼──────────────────┼──────────────────┼─────────────┘
          │                  │                  │
          │ (原生内嵌)       │ (HTTP/RPC)       │ (JSON-RPC/stdio)
          │                  │                  │
┌─────────┼──────────────────┼──────────────────┼─────────────┐
│         ▼                  ▼                  ▼              │
│                   Rust Agent 运行时                          │
│  ┌────────────────────────────────────────────────────┐     │
│  │  na-runtime: 会话管理 / 模型编排 / ReAct 协议      │     │
│  ├────────────────────────────────────────────────────┤     │
│  │  na-tools: 工具注册表 / 参数校验 / 输出管线        │     │
│  ├────────────────────────────────────────────────────┤     │
│  │  na-sandbox: 路径监狱 / 权限策略 / 资源限制        │     │
│  ├────────────────────────────────────────────────────┤     │
│  │  na-memory: checkpoint/回滚 / 审计日志 / RAG      │     │
│  ├────────────────────────────────────────────────────┤     │
│  │  na-story: 剧情状态 / 知识矩阵 / 伏笔追踪          │     │
│  └────────────────────────────────────────────────────┘     │
└──────────────────────────────────────────────────────────────┘
```

### Cargo Workspace 结构

- **na-common**：共享类型、错误归一化、取消令牌
- **na-sandbox**：沙箱实现（路径监狱、能力白名单、资源预算）
- **na-tools**：26 个内建工具 + 工具注册表 + 参数校验 + 输出处理管线
- **na-memory**：checkpoint/回滚、审计日志、BM25 长期记忆 RAG
- **na-story**：Story State 系统（角色状态、知识矩阵、伏笔追踪、硬约束）
- **na-runtime**：会话/上下文管理、agent 自循环、模型编排、ReAct 协议、Prompt 注入防护
- **na-host**：JSON-RPC 主机（GUI 后端进程） + 验证用二进制

**测试覆盖**：483 个单元测试 + 集成测试，`cargo test --workspace` 全绿 + `clippy` 零警告。

---

## 快速开始

### 前置要求

- **Rust** 1.75+（[安装指南](https://rustup.rs/)）
- **Node.js** 18+（桌面端/Web 端需要）
- **Flutter** 3.44+（仅移动端需要）

### 1. 运行核心层演示（推荐）

把它想象成一个「会用工具、有记忆、能自我约束的 AI 写作助手的大脑」：

- **会用工具**：读写文件、搜索、跑命令、抓网页、调用外部 MCP 工具、给小说做版本管理。
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
1. 打开应用 → 左侧「**供应商**」→ 新增你的 AI 服务商（OpenAI / DeepSeek / Claude 等）
2. 填入 **API Key** + 选择模型 → 测试连接 → 设为当前
3. 左侧「**策划**」→ 输入作品构思 → 与 AI 探讨 → 生成世界观/人物/大纲
4. 左侧「**创作**」→ 输入创作目标 + 章节标题 → 实时看 AI 运笔 → 产出章节
5. 左侧「**章节**」→ 改稿；「**版本**」→ 做版本管理（提交/分支/对比）

> 你的 API Key 只保存在本机（`%APPDATA%\com.novelgenerateteam.desktop\providers.json`）。

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

---

## 核心能力详解

### 🛠️ 工具系统

26 个内建工具覆盖全创作流程：

| 类别 | 工具 |
|------|------|
| **文件操作** | read_file, write_file, edit_file, delete_file, list_files, grep_files |
| **记忆管理** | memory_save, memory_recall, memory_archive, memory_delete, memory_classify |
| **版本控制** | checkpoint_create, checkpoint_list, checkpoint_restore, checkpoint_delete, git_commit, git_branch, git_diff |
| **网络访问** | web_fetch, mcp_call (外部 MCP 工具调用) |
| **搜索** | search_web, search_memory |
| **任务管理** | run_subagent, load_skill |

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

长篇创作中 AI 容易忘记之前的设定。Story State 系统通过**自动状态注入**解决：

```rust
pub struct StoryState {
    pub characters: HashMap<String, CharacterState>,  // 角色状态
    pub knowledge_matrix: KnowledgeMatrix,            // 知识矩阵（谁知道什么）
    pub foreshadows: Vec<ForeshadowTracker>,         // 伏笔追踪
    pub constraints: Vec<Constraint>,                // 硬约束（5 级严重性）
    pub timeline: Timeline,                          // 时间线
    pub world_state: WorldState,                     // 世界状态
}
```

**工作流程**：
1. 创作前：`prepare_context(current_chapter)` 提取当前章节需要的状态
2. 渲染成 Prompt 自动注入到会话开头（AI 在看到创作目标前先看到完整状态）
3. 创作后：分析输出并更新状态（角色、伏笔、时间线）

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

---

## 打包分发

### Windows 安装包

```bash
# 1. 构建桌面端
cd desktop-tauri
npm run tauri build -- --bundles nsis

# 2. 构建自定义安装器（单文件 exe）
cp "target/release/desktop-tauri.exe" "../installer/src-tauri/payload/NovelGenerateTeam.exe"
cd ../installer
npm install
npm run tauri build -- --no-bundle
```

产物：`installer/target/release/installer.exe`（约 23MB，双击即装，含自动更新检测）。

### Android APK

```bash
cd mobile
flutter build apk --release
```

产物：`build/app/outputs/flutter-apk/app-release.apk`（约 52MB）。

---

## 项目目录结构

```
Novel_Generate_Team/
├── core/                     # Rust 核心层（cargo workspace）
│   ├── Cargo.toml            # workspace 根
│   ├── crates/
│   │   ├── na-common/        # 共享类型、错误、取消令牌
│   │   ├── na-sandbox/       # 沙箱实现
│   │   ├── na-tools/         # 工具注册表 + 26 个内建工具
│   │   ├── na-memory/        # checkpoint/回滚 + 审计日志 + RAG
│   │   ├── na-story/         # Story State 系统
│   │   ├── na-runtime/       # Agent 运行时 + 模型编排
│   │   └── na-host/          # JSON-RPC 主机 + 验证二进制
│   └── tests/                # 跨 crate 集成测试
├── desktop-tauri/            # Tauri 桌面端（水墨国风，Rust 核心原生内嵌）
├── installer/                # 自定义无边框安装器（单文件 exe）
├── frontend/                 # Next.js Web 端
├── mobile/                   # Flutter 移动端（Android）
├── brand/                    # 品牌资源（图标/logo）
└── docs/                     # 技术文档
```

---

## 技术栈

| 层 | 技术 | 说明 |
|---|---|---|
| **核心运行时** | Rust (cargo workspace) | Agent 运行时 + 工具执行层 + 沙箱隔离 |
| **桌面端** | Tauri + React + Vite | Rust 核心原生内嵌，用系统 WebView2 |
| **Web 端** | Next.js 14 + TypeScript | 通过 Node 桥（JSON-RPC/stdio）调用核心 |
| **移动端** | Flutter 3.44 + Dart 3.12 | 通过 HTTP 连接后端 `/api/rpc` |
| **模型接入** | OpenAI API / Anthropic API | 支持流式输出（SSE） |
| **通信协议** | JSON-RPC 2.0 | stdio / WebSocket / HTTP |

---

## 贡献

欢迎贡献代码、报告问题或提出建议！

### 开发环境搭建

```bash
# 1. 克隆仓库
git clone https://github.com/young0081/Novel_Generate_Team.git
cd Novel_Generate_Team

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

- GitHub Issues: [提交问题](https://github.com/young0081/Novel_Generate_Team/issues)
- 项目作者: [@young0081](https://github.com/young0081)

---

<div align="center">

**如果这个项目对你有帮助，欢迎 Star ⭐**

Made with ❤️ and Rust 🦀

</div>
