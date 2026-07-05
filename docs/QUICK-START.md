# 🚀 Quick Start - 15分钟上手指南

> 这是一份让你快速开始实施「长程一致性增强方案」的最精简指南。

---

## ⚡ 3 分钟理解核心思路

### 问题
Agent 写长篇小说时会"忘记"设定，越写到后面问题越严重。

### 原因
设定没有**主动**注入到每次生成的上下文，依赖被动召回（经常失败）。

### 解决方案
```
创建结构化状态文件 (story_state.json)
    ↓
每次生成前，自动把关键设定注入上下文
    ↓
Agent 在"看到约束"的前提下创作
    ↓
生成后自动检查是否违反设定
```

---

## 📦 今天就开始（MVP 版本）

### Step 1: 创建新 crate (5 分钟)

```bash
cd core/crates
cargo new na-story --lib
cd na-story
```

编辑 `Cargo.toml`:
```toml
[dependencies]
na-common = { path = "../na-common" }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
```

### Step 2: 复制数据结构 (5 分钟)

创建 `src/state.rs`，复制以下代码：

```rust
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub type CharacterId = String;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoryState {
    pub characters: HashMap<CharacterId, CharacterState>,
    pub hard_constraints: Vec<Constraint>,
    pub foreshadows: Vec<ForeshadowTracker>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterState {
    pub name: String,
    pub core_traits: Vec<String>,
    pub current_status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Constraint {
    pub description: String,
    pub severity: Severity,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, PartialOrd)]
pub enum Severity {
    Critical = 5,
    High = 4,
    Medium = 3,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForeshadowTracker {
    pub description: String,
    pub planted_at: u32,
    pub status: ForeshadowStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ForeshadowStatus {
    Planted,
    Resolved,
}

impl Default for StoryState {
    fn default() -> Self {
        StoryState {
            characters: HashMap::new(),
            hard_constraints: vec![],
            foreshadows: vec![],
        }
    }
}
```

在 `src/lib.rs` 中：
```rust
pub mod state;
pub use state::*;
```

测试编译：
```bash
cargo build
```

### Step 3: 创建示例状态文件 (3 分钟)

在你的工作区创建 `story_state.json`:

```json
{
  "characters": {
    "char_001": {
      "name": "林惊羽",
      "core_traits": ["冷静", "重情义"],
      "current_status": "练气九层"
    }
  },
  "hard_constraints": [
    {
      "description": "林惊羽绝不会背叛朋友",
      "severity": "Critical"
    },
    {
      "description": "筑基期以下无法御剑飞行",
      "severity": "High"
    }
  ],
  "foreshadows": [
    {
      "description": "师傅临终时眼神看向北方",
      "planted_at": 1,
      "status": "Planted"
    }
  ]
}
```

### Step 4: 手动注入测试 (2 分钟)

在你下次创作时，手动在 prompt 前面加上：

```
# 当前剧情状态

## 核心角色
- **林惊羽**: 
  - 核心特征: 冷静、重情义
  - 当前处境: 练气九层

## ⚠️ 必须遵守的硬约束
- **[Critical]** 林惊羽绝不会背叛朋友
- **[High]** 筑基期以下无法御剑飞行

## 🌱 未回收伏笔
- 师傅临终时眼神看向北方 (第1章埋下)

---

{你的原始创作需求}
```

**立即验证**: 现在写一章，观察 Agent 是否更好地遵守设定了！

---

## 🎯 第一周目标

### Day 1 (今天)
- [x] 创建 `na-story` crate
- [x] 定义基础数据结构
- [x] 创建示例 `story_state.json`
- [x] 手动注入测试

### Day 2-3 (明后天)
- [ ] 实现 `StoryStateManager::open()` 和 `save()`
- [ ] 实现 `prepare_context()` 方法
- [ ] 实现 `render_state_sync_prompt()` 函数
- [ ] 自动化注入（不再手动复制粘贴）

### Day 4-5 (本周末)
- [ ] 添加 GUI 状态编辑页面
- [ ] 实现基础一致性检查
- [ ] 完整测试 5 章流程

---

## 📝 核心代码片段（复制即用）

### StoryStateManager (src/manager.rs)

```rust
use crate::state::*;
use std::fs;
use std::path::Path;

pub struct StoryStateManager {
    pub state: StoryState,
}

impl StoryStateManager {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, Box<dyn std::error::Error>> {
        let state = if path.as_ref().exists() {
            let content = fs::read_to_string(path)?;
            serde_json::from_str(&content)?
        } else {
            StoryState::default()
        };
        Ok(StoryStateManager { state })
    }
    
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), Box<dyn std::error::Error>> {
        let content = serde_json::to_string_pretty(&self.state)?;
        fs::write(path, content)?;
        Ok(())
    }
    
    pub fn render_prompt(&self) -> String {
        let mut prompt = String::new();
        prompt.push_str("# 当前剧情状态\n\n");
        
        // 角色
        if !self.state.characters.is_empty() {
            prompt.push_str("## 核心角色\n");
            for char in self.state.characters.values() {
                prompt.push_str(&format!("- **{}**: {}\n", 
                    char.name, 
                    char.core_traits.join(", ")
                ));
            }
            prompt.push('\n');
        }
        
        // 约束
        if !self.state.hard_constraints.is_empty() {
            prompt.push_str("## ⚠️ 必须遵守的硬约束\n");
            for c in &self.state.hard_constraints {
                prompt.push_str(&format!("- **[{:?}]** {}\n", c.severity, c.description));
            }
            prompt.push('\n');
        }
        
        prompt.push_str("---\n\n");
        prompt
    }
}
```

### 使用示例

```rust
// 加载状态
let mgr = StoryStateManager::open("workspace/story_state.json")?;

// 生成 prompt
let state_prompt = mgr.render_prompt();

// 注入到你的 session
session.push(Message::system(state_prompt));

// 继续正常的 goal loop...
```

---

## ✅ 验证是否生效

### 测试方法

1. **创建测试设定**:
   - 角色："主角"，特征："冷静"
   - 约束："主角不会杀无辜"

2. **写 3 章正常剧情**

3. **第 4 章故意诱导违规**:
   ```
   prompt: "主角暴怒，屠杀了整个村庄"
   ```

4. **观察结果**:
   - ✅ 生成的内容避开了屠杀，或者主角表现出挣扎
   - ❌ 主角直接屠村，无任何犹豫 → 状态注入可能失败

### 成功标志

- Agent 生成内容符合角色特征
- 硬约束被遵守
- 不再出现"突然忘记设定"的情况

---

## 🆘 遇到问题？

### 编译失败
→ 检查 `Cargo.toml` 依赖路径是否正确

### 状态文件加载失败
→ 检查 JSON 格式，使用在线工具验证

### 注入后仍然忘设定
→ 检查 prompt 是否真的被加入 session
→ 检查约束描述是否够明确

### 想要更多功能
→ 查看完整文档 `INDEX.md`

---

## 📚 下一步

完成上述步骤后，你已经有了一个**可用的 MVP**！

接下来可以：

1. **添加 GUI**: 让用户通过界面编辑状态
2. **自动化检查**: 生成后检测是否违反约束
3. **状态自动提取**: 从生成内容自动更新状态

详细步骤见 `implementation-checklist.md`

---

## 🎁 奖励：一键测试脚本

创建 `test_consistency.sh`:

```bash
#!/bin/bash

# 创建测试工作区
mkdir -p test_workspace

# 生成测试状态文件
cat > test_workspace/story_state.json << 'EOF'
{
  "characters": {
    "char_001": {
      "name": "测试主角",
      "core_traits": ["善良", "谨慎"],
      "current_status": "初始状态"
    }
  },
  "hard_constraints": [
    {
      "description": "主角绝不会伤害无辜",
      "severity": "Critical"
    }
  ],
  "foreshadows": []
}
EOF

echo "✅ 测试环境已创建在 test_workspace/"
echo "现在可以在这个工作区测试你的实现了！"
```

运行：
```bash
chmod +x test_consistency.sh
./test_consistency.sh
```

---

## 🎉 恭喜！

你已经掌握了核心概念并搭建了基础框架。

**记住核心思路**：
- 结构化存储设定 (JSON)
- 每次生成前主动注入 (Prompt)
- 生成后检查一致性 (Guard)

现在开始你的实施之旅吧！🚀

---

**需要帮助？**
- 技术细节 → 查看完整文档 `INDEX.md`
- 实施步骤 → 查看 `implementation-checklist.md`
- 快速参考 → 查看 `README-CONSISTENCY-ENHANCEMENT.md`

祝你成功！💪

