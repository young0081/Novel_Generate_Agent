//! End-to-end, fully-offline demonstration of the Novel Generate Agent core
//! (Phase-1 base + Phase-2 hardening).
//!
//! Driven entirely by scripted mock models, it shows the whole stack working
//! together: a fully-wired engine with lifecycle hooks, skills, a subagent, loop
//! observability and a `writer.md` style guide; one complete agent loop that
//! loads a skill, delegates a side task to a subagent, writes a chapter and saves
//! a character memory; then memory recall, a checkpoint + rollback, and a hook
//! blocking a dangerous tool.
//!
//! Run with:  `cargo run -p na-host --bin demo`

use std::sync::Arc;

use na_common::{json, Result};
use na_host::Engine;
use na_runtime::{
    CompletionResponse, LoopHookRegistry, MockProvider, ModelProvider, Protocol, RecordingLoopHook,
    Skill, SkillRegistry, ToolCallRequest,
};
use na_tools::{DenyToolHook, HookRegistry, LoggingHook};

fn line() {
    println!("────────────────────────────────────────────────────────");
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut root = std::env::temp_dir();
    root.push(format!("novel_demo_{}", na_common::next_id("ws")));

    println!("📚 Novel Generate Agent —— 核心层端到端演示（含加固能力）");
    line();
    println!("工作区: {}", root.display());

    // --- skills: a reusable three-act outline playbook ---
    let mut skills = SkillRegistry::new();
    skills.register(Skill::new(
        "three_act",
        "用三幕式结构搭建故事大纲。",
        "第一幕·建置：交代主角、世界与渴望。\n\
         第二幕·对抗：升级冲突，逼主角付出代价。\n\
         第三幕·结局：高潮对决与情感收束。",
        vec!["write_file".to_string()],
    ));
    let skills = Arc::new(skills);

    // --- tool-lifecycle hooks: audit-logging + block the dangerous `shell` tool ---
    let mut hooks = HookRegistry::new();
    hooks.register(Arc::new(LoggingHook::new()));
    hooks.register(Arc::new(DenyToolHook::new(["shell"])));
    let hooks = Arc::new(hooks);

    // --- loop observability: record every step ---
    let recorder = RecordingLoopHook::new();
    let mut loop_hooks = LoopHookRegistry::new();
    loop_hooks.register(Arc::new(recorder.clone()));
    let loop_hooks = Arc::new(loop_hooks);

    // --- the subagent's own scripted model: it writes a side bio then finishes ---
    let subagent_provider: Arc<dyn ModelProvider> = Arc::new(MockProvider::from_responses(vec![
        CompletionResponse::tool_call(ToolCallRequest::new(
            "write_file",
            json!({ "path": "book/支线-小传.md", "content": "沈霜：林惊羽少年时的旧友，后因立场相左而决裂。" }),
        )),
        CompletionResponse::answer("支线人物小传已写好。"),
    ]));

    let engine = Engine::full(&root, subagent_provider, skills, hooks, loop_hooks)?;
    println!(
        "已加载 {} 个工具（19 内置 + 3 运行时：技能/子代理）。",
        engine.registry.len()
    );

    // The author's standing style guide — injected into every run.
    std::fs::write(
        engine.workspace_root().join("writer.md"),
        "第三人称限制视角；多用短句；冷峻克制的笔调；避免陈词滥调。",
    )
    .map_err(na_common::CoreError::from)?;
    line();

    // --- main agent loop: load a skill, delegate to a subagent, write, remember ---
    let chapter = "第一章 霜寒初现\n\n北境的风像刀子。林惊羽立于断崖，握紧名为「霜寒」的长剑，眼神平静而锋利。狼群围拢，他不退半步。";
    let responses = vec![
        CompletionResponse::tool_call(ToolCallRequest::new(
            "skill_load",
            json!({ "name": "three_act" }),
        )),
        CompletionResponse::tool_call(ToolCallRequest::new(
            "spawn_subagent",
            json!({ "goal": "为支线人物沈霜写一段小传", "title": "支线小传" }),
        )),
        CompletionResponse::tool_call(ToolCallRequest::new(
            "write_file",
            json!({ "path": "book/ch1.md", "content": chapter }),
        )),
        CompletionResponse::tool_call(ToolCallRequest::new(
            "memory_save",
            json!({
                "kind": "character", "title": "林惊羽",
                "summary": "冷静果敢的年轻剑客，本书主角，持长剑「霜寒」。",
                "content": "出身北境寒门，沉静，临危不乱，招式快准狠。",
                "tags": ["主角", "剑客", "北境"], "importance": 5
            }),
        )),
        CompletionResponse::answer(
            "第一章已写好；并通过技能规划了三幕结构、用子代理补了支线人物小传。",
        ),
    ];

    println!("▶ 运行 agent loop（载入技能 → 派生子代理 → 写章节 → 存记忆）……");
    let (outcome, session) = engine
        .run_goal_scripted(
            "写好第一章，并补全支线人物",
            "北境剑歌",
            Protocol::NativeToolCall,
            responses,
        )
        .await?;

    println!(
        "  停止原因: {}  | 步数: {}  | 成功: {}  | loop 钩子观测到 {} 步",
        outcome.stopped_reason.as_str(),
        outcome.steps,
        outcome.stopped_reason.is_success(),
        recorder.step_count(),
    );
    println!(
        "  最终回答: {}",
        outcome.final_answer.as_deref().unwrap_or("(无)")
    );
    let profile_injected = session
        .history()
        .iter()
        .any(|m| m.content.contains("作者风格指南") && m.content.contains("短句"));
    println!(
        "  writer.md 文风已注入上下文: {}",
        if profile_injected {
            "✅ 是"
        } else {
            "❌ 否"
        }
    );
    line();

    // The subagent's side effect landed on disk.
    let bio = engine
        .invoke_tool("read_file", json!({ "path": "book/支线-小传.md" }))
        .await;
    println!("🤝 子代理产出 book/支线-小传.md：");
    println!("  {}", bio.content.trim());
    line();

    // The chapter and the memory.
    let read = engine
        .invoke_tool("read_file", json!({ "path": "book/ch1.md" }))
        .await;
    let preview: String = read.content.chars().take(48).collect();
    println!("📖 主章节 book/ch1.md：\n  {preview}…");
    let recall = engine
        .invoke_tool(
            "memory_recall",
            json!({ "query": "北境 剑客 主角", "k": 2 }),
        )
        .await;
    println!("🧠 记忆检索（只返摘要）：\n  {}", recall.content.trim());
    line();

    // Checkpoint → destructive edit → rollback.
    let ckpt = engine
        .invoke_tool("checkpoint_create", json!({ "label": "初稿 v1" }))
        .await;
    let ckpt_id = ckpt.data["id"].as_str().unwrap_or("").to_string();
    engine
        .invoke_tool(
            "write_file",
            json!({ "path": "book/ch1.md", "content": "【手滑删光了】" }),
        )
        .await;
    engine
        .invoke_tool("checkpoint_restore", json!({ "id": ckpt_id }))
        .await;
    let restored = engine
        .invoke_tool("read_file", json!({ "path": "book/ch1.md" }))
        .await;
    println!(
        "💾↩️  快照→误删→回滚：{}",
        if restored.content.contains("霜寒初现") {
            "✅ 手稿已完整恢复"
        } else {
            "❌ 恢复失败"
        }
    );
    line();

    // A lifecycle hook blocks the dangerous `shell` tool before it can run.
    let blocked = engine
        .invoke_tool("shell", json!({ "command": "echo hi" }))
        .await;
    println!(
        "🛡️  DenyToolHook 拦截 shell 工具：{}（{}）",
        if !blocked.ok {
            "✅ 已拦截，未执行"
        } else {
            "❌ 未拦截"
        },
        blocked.content.trim()
    );
    line();

    // Audit trail.
    let audit_path = engine.workspace_root().join(".na").join("audit.jsonl");
    let audit_count = std::fs::read_to_string(&audit_path)
        .map(|s| s.lines().filter(|l| !l.trim().is_empty()).count())
        .unwrap_or(0);
    println!("🧾 审计日志记录了 {audit_count} 条事件（工具调用、hook 决定、回滚等都落盘）。");
    line();

    println!("✅ 演示完成。覆盖：agent 自循环 + 工具完整生命周期 + 沙箱/权限 + 输出处理");
    println!(
        "   + 记忆 RAG + checkpoint 回滚 + 审计日志 + hooks + skills + subagents + writer.md。"
    );

    Ok(())
}
