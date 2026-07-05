//! Cross-crate integration tests: they exercise the whole core stack together
//! (sandbox → tools → memory → runtime → host) the way the GUI will.

use na_common::json;
use na_host::{dispatch, handle_line, CompletionResponse, Engine, Protocol, ToolCallRequest};

fn temp_root(tag: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("na_host_it_{}_{}", tag, na_common::next_id("t")));
    p
}

/// A full scripted agent run writes a chapter and a memory, end to end.
#[tokio::test]
async fn full_goal_run_writes_chapter_and_memory() {
    let engine = Engine::new(temp_root("full")).unwrap();

    let responses = vec![
        CompletionResponse::tool_call(ToolCallRequest::new(
            "write_file",
            json!({ "path": "book/ch1.md", "content": "第一章\n林惊羽提剑而立。" }),
        )),
        CompletionResponse::tool_call(ToolCallRequest::new(
            "memory_save",
            json!({
                "kind": "character",
                "title": "林惊羽",
                "summary": "年轻剑客，主角。",
                "content": "持剑霜寒，沉着冷静。",
                "tags": ["主角", "剑客"],
                "importance": 5
            }),
        )),
        CompletionResponse::answer("第一章已完成，主角已入库。"),
    ];

    let (outcome, session) = engine
        .run_goal_scripted("写第一章", "北境剑歌", Protocol::NativeToolCall, responses)
        .await
        .unwrap();

    assert!(outcome.stopped_reason.is_success());
    assert_eq!(
        outcome.final_answer.as_deref(),
        Some("第一章已完成，主角已入库。")
    );
    assert!(session.len() >= 4);

    // File really exists.
    let read = engine
        .invoke_tool("read_file", json!({ "path": "book/ch1.md" }))
        .await;
    assert!(read.ok);
    assert!(read.content.contains("林惊羽提剑而立"));

    // Memory really recalls.
    let recall = engine
        .invoke_tool("memory_recall", json!({ "query": "剑客", "k": 3 }))
        .await;
    assert!(recall.ok);
    assert!(recall.data["count"].as_u64().unwrap() >= 1);
}

/// Checkpoint rollback restores the manuscript byte-exactly but leaves long-term
/// memory and the audit trail intact (validates the `.na` exclusion).
#[tokio::test]
async fn rollback_restores_manuscript_but_keeps_memory() {
    let engine = Engine::new(temp_root("rollback")).unwrap();

    engine
        .invoke_tool(
            "write_file",
            json!({ "path": "book/ch1.md", "content": "原始正文 v1" }),
        )
        .await;
    let saved = engine
        .invoke_tool(
            "memory_save",
            json!({ "kind": "setting", "title": "北境", "summary": "极寒之地", "content": "终年风雪。" }),
        )
        .await;
    assert!(saved.ok);

    let ckpt = engine
        .invoke_tool("checkpoint_create", json!({ "label": "v1" }))
        .await;
    let id = ckpt.data["id"].as_str().unwrap().to_string();

    // Destructive overwrite.
    engine
        .invoke_tool(
            "write_file",
            json!({ "path": "book/ch1.md", "content": "毁了" }),
        )
        .await;
    let broken = engine
        .invoke_tool("read_file", json!({ "path": "book/ch1.md" }))
        .await;
    assert!(broken.content.contains("毁了"));

    // Restore.
    let restore = engine
        .invoke_tool("checkpoint_restore", json!({ "id": id }))
        .await;
    assert!(restore.ok, "{}", restore.content);

    let restored = engine
        .invoke_tool("read_file", json!({ "path": "book/ch1.md" }))
        .await;
    assert!(
        restored.content.contains("原始正文 v1"),
        "manuscript must roll back"
    );

    // Memory still there after the rollback.
    let recall = engine
        .invoke_tool("memory_recall", json!({ "query": "极寒", "k": 3 }))
        .await;
    assert!(
        recall.data["count"].as_u64().unwrap() >= 1,
        "memory must survive a manuscript rollback"
    );
}

/// The sandbox blocks path-traversal escapes through the normal tool path.
#[tokio::test]
async fn sandbox_blocks_escape_through_engine() {
    let engine = Engine::new(temp_root("escape")).unwrap();
    let r = engine
        .invoke_tool("read_file", json!({ "path": "../../../../etc/passwd" }))
        .await;
    assert!(!r.ok, "path escape must fail");
    assert!(r.content.starts_with("[error:"));
}

/// The ReAct text protocol drives a full run end to end.
#[tokio::test]
async fn react_protocol_end_to_end() {
    let engine = Engine::new(temp_root("react")).unwrap();
    let responses = vec![
        CompletionResponse::react(
            "Thought: 先写开篇\nAction: write_file\nAction Input: {\"path\": \"ch1.md\", \"content\": \"起。\"}",
        ),
        CompletionResponse::react("Thought: 完成\nFinal Answer: 写好了。"),
    ];
    let (outcome, _session) = engine
        .run_goal_scripted("用 ReAct 写一段", "react书", Protocol::ReActText, responses)
        .await
        .unwrap();
    assert!(outcome.stopped_reason.is_success());
    assert_eq!(outcome.final_answer.as_deref(), Some("写好了。"));

    let read = engine
        .invoke_tool("read_file", json!({ "path": "ch1.md" }))
        .await;
    assert!(read.ok);
    assert!(read.content.contains("起。"));
}

/// The JSON-RPC surface round-trips over `handle_line` (what the GUI uses).
#[tokio::test]
async fn rpc_line_protocol_round_trip() {
    let engine = Engine::new(temp_root("rpc")).unwrap();

    // ping
    let pong = handle_line(&engine, r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#)
        .await
        .unwrap();
    let v: na_common::Json = serde_json::from_str(&pong).unwrap();
    assert_eq!(v["result"], "pong");
    assert_eq!(v["id"], 1);

    // list_tools
    let tools = handle_line(&engine, r#"{"id":2,"method":"list_tools"}"#)
        .await
        .unwrap();
    let v: na_common::Json = serde_json::from_str(&tools).unwrap();
    assert_eq!(v["result"].as_array().unwrap().len(), 23);

    // write via direct-tool method, then read
    let w = handle_line(
        &engine,
        r#"{"id":3,"method":"write_file","params":{"path":"a.md","content":"内容"}}"#,
    )
    .await
    .unwrap();
    let v: na_common::Json = serde_json::from_str(&w).unwrap();
    assert_eq!(v["result"]["ok"], true);

    let r = dispatch(&engine, "read_file", json!({ "path": "a.md" }))
        .await
        .unwrap();
    assert!(r["content"].as_str().unwrap().contains("内容"));
}

/// Cancellation makes the engine's context refuse further work cleanly.
#[tokio::test]
async fn cancellation_is_observed() {
    let engine = Engine::new(temp_root("cancel")).unwrap();
    engine.cancel();
    // A cancelled context surfaces a cancelled error result rather than running.
    let r = engine
        .invoke_tool("write_file", json!({ "path": "x.md", "content": "y" }))
        .await;
    assert!(!r.ok);
}

/// Phase-2: skills + subagents are reachable through a fully-wired engine.
#[tokio::test]
async fn full_engine_skill_and_subagent() {
    use na_runtime::{LoopHookRegistry, MockProvider, ModelProvider, Skill, SkillRegistry};
    use na_tools::HookRegistry;
    use std::sync::Arc;

    let mut skills = SkillRegistry::new();
    skills.register(Skill::new(
        "outline",
        "outline a story",
        "Three acts.",
        vec!["write_file".to_string()],
    ));

    let provider: Arc<dyn ModelProvider> = Arc::new(MockProvider::from_responses(vec![
        CompletionResponse::tool_call(ToolCallRequest::new(
            "write_file",
            json!({ "path": "sub.md", "content": "child wrote this" }),
        )),
        CompletionResponse::answer("child done"),
    ]));

    let engine = Engine::full(
        temp_root("fulleng"),
        provider,
        Arc::new(skills),
        Arc::new(HookRegistry::new()),
        Arc::new(LoopHookRegistry::new()),
    )
    .unwrap();
    assert_eq!(engine.registry.len(), 26);

    // skill_load returns the playbook body (the instructions).
    let s = engine
        .invoke_tool("skill_load", json!({ "name": "outline" }))
        .await;
    assert!(s.ok);
    assert_eq!(s.content, "Three acts.");

    // spawn_subagent runs a bounded child and returns a SUMMARY (not the transcript).
    let sub = engine
        .invoke_tool(
            "spawn_subagent",
            json!({ "goal": "write a file", "title": "child" }),
        )
        .await;
    assert!(sub.ok, "{}", sub.content);
    assert_eq!(sub.data["success"], true);

    // The child's side effect really landed on disk.
    let read = engine
        .invoke_tool("read_file", json!({ "path": "sub.md" }))
        .await;
    assert!(read.content.contains("child wrote this"));
}

/// Phase-2: a tool-lifecycle hook blocks a tool before it executes.
#[tokio::test]
async fn tool_hook_blocks_a_tool() {
    use na_runtime::{LoopHookRegistry, MockProvider, ModelProvider, SkillRegistry};
    use na_tools::{DenyToolHook, HookRegistry};
    use std::sync::Arc;

    let mut hooks = HookRegistry::new();
    hooks.register(Arc::new(DenyToolHook::new(["read_file"])));
    let provider: Arc<dyn ModelProvider> = Arc::new(MockProvider::from_responses(vec![
        CompletionResponse::answer("x"),
    ]));

    let engine = Engine::full(
        temp_root("hookblock"),
        provider,
        Arc::new(SkillRegistry::new()),
        Arc::new(hooks),
        Arc::new(LoopHookRegistry::new()),
    )
    .unwrap();

    // Writing is allowed.
    let w = engine
        .invoke_tool("write_file", json!({ "path": "a.md", "content": "hi" }))
        .await;
    assert!(w.ok);

    // read_file is blocked by the hook (security_blocked) and never executes.
    let r = engine
        .invoke_tool("read_file", json!({ "path": "a.md" }))
        .await;
    assert!(!r.ok);
    assert!(r.content.contains("[error:"));
    assert_eq!(r.data["code"], "security_blocked");
}
