//! Tauri desktop backend — the Rust core embedded directly as the app's native
//! backend (no separate process, no Node server). Commands are called from the
//! web UI via `invoke(...)` and dispatch into the shared [`Engine`].

use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};

use na_host::{outcome_to_json, CompletionResponse, CoreError, Engine, Protocol};
use na_library::{
    KnowledgeBaseMeta, KnowledgeEntry, KnowledgeHit, KnowledgeKind, KnowledgeStore, WorkMeta,
    WorkStore, WorkSummary,
};
use na_runtime::{
    test_connection, CompletionRequest, GoalLoop, LoopHook, LoopHookRegistry, LoopOutcome, Message,
    ModelProvider, ProjectProfile, ProviderConfig, ProviderSettings, ProviderStore, Session,
    SessionId, SessionRecord, SessionStore, SessionSummary,
};
use na_sandbox::Capability;
use na_tools::{ResultMeta, ToolSpec};
use serde_json::Value as Json;
use tauri::{Emitter, Manager, State};

/// The mutable application state shared across all commands.
///
/// Unlike the old single-`Arc<Engine>` model, the active workspace can now be
/// switched at runtime (multi-work support): swapping the active work rebuilds
/// the engine pointed at that work's private workspace directory, giving total
/// isolation between novels (manuscript, memory, checkpoints, story-state, and
/// knowledge bases are all per-work).
struct AppState {
    /// The engine for the currently-active work. Swapped on work switch.
    engine: RwLock<Arc<Engine>>,
    /// The library of all works + the active selection.
    works: Mutex<WorkStore>,
}

impl AppState {
    /// A cloned handle to the active work's engine (cheap Arc clone).
    fn engine(&self) -> Arc<Engine> {
        self.engine.read().unwrap().clone()
    }
}

/// The active work's sessions directory (created on demand).
fn active_sessions_dir(state: &AppState) -> Result<PathBuf, String> {
    let works = state.works.lock().unwrap();
    let w = works
        .active()
        .ok_or_else(|| "当前没有活动作品".to_string())?;
    let dir = w.sessions_dir.clone();
    drop(works);
    std::fs::create_dir_all(&dir).ok();
    Ok(dir)
}

/// The active work's knowledge directory (created on demand).
fn active_knowledge_dir(state: &AppState) -> Result<PathBuf, String> {
    let works = state.works.lock().unwrap();
    let w = works
        .active()
        .ok_or_else(|| "当前没有活动作品".to_string())?;
    let dir = w.knowledge_dir.clone();
    drop(works);
    std::fs::create_dir_all(&dir).ok();
    Ok(dir)
}

/// Write a default `writer.md` into `workspace_dir` if none exists yet.
///
/// This is the single most important intervention for reliable chapter saving:
/// the AI reads `writer.md` as a system-level style guide before every run.
/// Without explicit instructions it tends to dump the chapter text into its
/// Final Answer (which never touches disk) instead of calling `write_file`.
fn ensure_default_writer_md(workspace_dir: &std::path::Path) {
    let path = workspace_dir.join("writer.md");
    if path.exists() {
        return; // author's custom guide takes precedence — never overwrite
    }
    let default = r#"# 写作规范（系统默认）

## 【最高优先级】章节保存规则

**每次完成章节内容后，必须调用 `write_file` 工具将内容保存到磁盘。**

- 路径格式：`book/<章节名>.md`（例如 `book/第一章.md`、`book/ch01.md`）
- 禁止将章节正文放在 Final Answer 里——Final Answer 只用于报告"已完成"状态
- 正确流程：①构思 → ②运笔 → ③调用 write_file 保存 → ④Final Answer 报告完成

示例（正确）：
```
write_file({"path": "book/第一章.md", "content": "第一章 ..."})
Final Answer: 已完成，章节已保存至 book/第一章.md
```

## 写作风格

- 用流畅自然的中文叙述
- 注重人物情感与场景描写
- 保持前后文设定一致
"#;
    let _ = std::fs::create_dir_all(workspace_dir);
    let _ = std::fs::write(&path, default.as_bytes());
}

/// Rebuild the engine to point at the currently-active work's workspace and swap
/// it in. Call after changing which work is active.
fn rebuild_engine_to_active(state: &AppState) -> Result<(), String> {
    let workspace_dir = {
        let works = state.works.lock().unwrap();
        works
            .active()
            .ok_or_else(|| "当前没有活动作品".to_string())?
            .workspace_dir
            .clone()
    };
    ensure_default_writer_md(&workspace_dir);
    let engine = Engine::new(&workspace_dir).map_err(|e| e.to_string())?;
    *state.engine.write().unwrap() = Arc::new(engine);
    Ok(())
}

/// Render a set of knowledge-base hits into a system steering message so the AI
/// stays faithful to canon while writing (RAG injection).
fn render_knowledge_prompt(hits: &[KnowledgeHit]) -> String {
    let mut s = String::from("# 知识库参考（设定准绳）\n\n");
    s.push_str("以下是与当前创作目标相关的设定资料，请在创作时严格遵循，避免与之矛盾：\n\n");
    for h in hits {
        s.push_str(&format!(
            "- **{}**（{}）：{}\n",
            h.entry.title, h.kb_name, h.entry.content
        ));
    }
    s.push_str("\n**重要**：若与上述设定冲突，以上述设定为准。\n");
    s
}

/// A loop observer that streams each agent step to the UI as `agent-step` events,
/// so the 创作 screen can show the AI thinking / calling tools live.
struct TauriLoopHook {
    app: tauri::AppHandle,
}

impl LoopHook for TauriLoopHook {
    fn name(&self) -> &str {
        "tauri-stream"
    }

    fn on_step_start(&self, step: u32, session: &Session) {
        let _ = self.app.emit(
            "agent-step",
            serde_json::json!({ "phase": "step", "step": step, "messages": session.len() }),
        );
    }

    fn on_model_delta(&self, step: u32, delta: &str) {
        let _ = self.app.emit(
            "agent-step",
            serde_json::json!({ "phase": "delta", "step": step, "delta": delta }),
        );
    }

    fn on_model_response(&self, step: u32, resp: &CompletionResponse) {
        let calls: Vec<Json> = resp
            .tool_calls
            .iter()
            .map(|c| serde_json::json!({ "name": c.name, "args": c.args }))
            .collect();
        let _ = self.app.emit(
            "agent-step",
            serde_json::json!({
                "phase": "model",
                "step": step,
                "text": resp.text,
                "tool_calls": calls,
            }),
        );
    }

    fn on_finish(&self, outcome: &LoopOutcome) {
        let _ = self.app.emit(
            "agent-step",
            serde_json::json!({
                "phase": "finish",
                "reason": outcome.stopped_reason.as_str(),
                "success": outcome.stopped_reason.is_success(),
                "steps": outcome.steps,
                "final": outcome.final_answer,
            }),
        );
    }
}

/// Path to the provider-config file (under the OS app-data dir).
fn providers_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| CoreError::internal(e.to_string()).to_string())?;
    std::fs::create_dir_all(&dir).ok();
    Ok(dir.join("providers.json"))
}

/// Get the full provider configuration (all providers + active selection).
#[tauri::command]
fn providers_get(app: tauri::AppHandle) -> Result<ProviderSettings, String> {
    let path = providers_path(&app)?;
    Ok(ProviderStore::open(&path)
        .map_err(|e| e.to_string())?
        .settings()
        .clone())
}

/// Add or update a provider; returns the updated settings.
#[tauri::command]
fn providers_save(app: tauri::AppHandle, config: ProviderConfig) -> Result<ProviderSettings, String> {
    let path = providers_path(&app)?;
    let mut store = ProviderStore::open(&path).map_err(|e| e.to_string())?;
    store.upsert(config).map_err(|e| e.to_string())?;
    Ok(store.settings().clone())
}

/// Remove a provider; returns the updated settings.
#[tauri::command]
fn providers_delete(app: tauri::AppHandle, id: String) -> Result<ProviderSettings, String> {
    let path = providers_path(&app)?;
    let mut store = ProviderStore::open(&path).map_err(|e| e.to_string())?;
    store.remove(&id).map_err(|e| e.to_string())?;
    Ok(store.settings().clone())
}

/// Choose the active provider + model; returns the updated settings.
#[tauri::command]
fn providers_set_active(
    app: tauri::AppHandle,
    provider_id: String,
    model: String,
) -> Result<ProviderSettings, String> {
    let path = providers_path(&app)?;
    let mut store = ProviderStore::open(&path).map_err(|e| e.to_string())?;
    store
        .set_active(&provider_id, &model)
        .map_err(|e| e.to_string())?;
    Ok(store.settings().clone())
}

/// Test a provider/model by sending a tiny request; returns the reply text.
#[tauri::command]
async fn provider_test(config: ProviderConfig, model: String) -> Result<String, String> {
    test_connection(&config, &model).await.map_err(|e| e.to_string())
}

/// Directory holding the active work's persisted sessions.
fn sessions_dir(_app: &tauri::AppHandle, state: &AppState) -> Result<PathBuf, String> {
    active_sessions_dir(state)
}

/// Drive a real agent loop using the active provider + model.
///
/// When `session_id` names an existing saved session, the run CONTINUES it
/// (its full prior context is loaded, so the AI writes on from where it left
/// off). Otherwise a fresh writing session is started, seeded with the author's
/// standing instructions. Either way the session is persisted afterward so it
/// can be browsed and resumed from the 会话 library.
#[tauri::command]
async fn run_goal_live(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    goal: String,
    title: String,
    session_id: Option<String>,
) -> Result<Json, String> {
    let path = providers_path(&app)?;
    let store = ProviderStore::open(&path).map_err(|e| e.to_string())?;
    let agent_protocol = store
        .active()
        .map(|(cfg, _model)| cfg.agent_protocol())
        .unwrap_or(Protocol::NativeToolCall);
    let provider = store.build_active().map_err(|e| e.to_string())?;
    let engine = state.engine();
    let sessions = sessions_dir(&app, &state)?;
    let knowledge = active_knowledge_dir(&state)?;
    let active_work_id = state.works.lock().unwrap().active_id().map(|s| s.to_string());
    let sess_store = SessionStore::open(&sessions).map_err(|e| e.to_string())?;

    // Continue a saved session, or start a fresh one seeded with writer.md /
    // outline.md so the AI keeps the author's standing voice.
    let mut session = match session_id.as_deref().and_then(|id| sess_store.get(id).ok()) {
        Some(rec) => rec.session,
        None => {
            let mut s = Session::new(&title);
            for m in ProjectProfile::load(engine.ctx.jail.root()).system_messages() {
                s.push(m);
            }
            s
        }
    };

    // RAG: inject relevant knowledge-base facts so the AI stays on-setting.
    if let Ok(kb_store) = KnowledgeStore::open(&knowledge) {
        if let Ok(hits) = kb_store.search_active(&format!("{title} {goal}"), 8) {
            if !hits.is_empty() {
                session.push(Message::system(render_knowledge_prompt(&hits)));
            }
        }
    }

    // Load and inject story state if it exists (consistency enhancement).
    let workspace_root = engine.ctx.jail.root();
    let state_path = workspace_root.join("story_state.json");
    if state_path.exists() {
        if let Ok(mgr) = na_runtime::StoryStateManager::open(&state_path) {
            // Determine chapter number (from meta or session length heuristic)
            let chapter_num = mgr.state.meta.last_chapter.saturating_add(1);
            let ctx_pkg = mgr.prepare_context(chapter_num);
            let state_prompt = na_runtime::render_state_sync_prompt(&ctx_pkg);
            // Inject as system message so it appears before the goal
            session.push(Message::system(state_prompt));
        }
    }

    // Stream each step to the UI.
    let mut hooks = LoopHookRegistry::new();
    hooks.register(Arc::new(TauriLoopHook { app: app.clone() }));

    let outcome = GoalLoop::with_protocol(agent_protocol)
        .loop_hooks(Arc::new(hooks))
        .run(&goal, &mut session, &provider, &engine.registry, &engine.ctx)
        .await;

    // Persist whatever the session became (even on error) so context isn't lost.
    let _ = sess_store.save(&SessionRecord {
        session: session.clone(),
        kind: "writing".to_string(),
        goal: Some(goal.clone()),
    });

    // Bump the work's recency so the library sorts it to the top.
    if let Some(id) = active_work_id {
        let _ = state.works.lock().unwrap().touch(&id);
    }

    let outcome = outcome.map_err(|e| e.to_string())?;

    // ── Auto-save fallback ────────────────────────────────────────────────────
    // If the AI dumped the chapter text straight into its Final Answer instead
    // of calling write_file (the most common failure mode), and that text is
    // substantial (> 200 chars), we save it automatically so the content is
    // never silently lost. We only do this when no write_file call is found in
    // the session transcript (i.e. the AI never saved the file itself).
    let auto_saved_path: Option<String> = {
        let final_text = outcome.final_answer.as_deref().unwrap_or("").trim();
        let already_saved = session
            .history()
            .iter()
            .any(|m| m.tool_call.as_ref().map(|c| c.name == "write_file").unwrap_or(false));
        if !already_saved && final_text.chars().count() > 200 {
            let safe_title: String = title
                .chars()
                .map(|c| if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '_' } else { c })
                .collect();
            let rel_path = format!("book/{safe_title}.md");
            let abs_path = engine.ctx.jail.root().join("book").join(format!("{safe_title}.md"));
            let _ = std::fs::create_dir_all(abs_path.parent().unwrap());
            match std::fs::write(&abs_path, final_text.as_bytes()) {
                Ok(_) => Some(rel_path),
                Err(_) => None,
            }
        } else {
            None
        }
    };

    let mut outcome_json = outcome_to_json(&outcome);
    if let (Some(path), Some(obj)) = (&auto_saved_path, outcome_json.as_object_mut()) {
        obj.insert("auto_saved_path".to_string(), serde_json::Value::String(path.clone()));
    }

    Ok(serde_json::json!({
        "outcome": outcome_json,
        "session": serde_json::to_value(&session).map_err(|e| e.to_string())?,
    }))
}

/// Liveness check.
#[tauri::command]
fn ping() -> &'static str {
    "pong"
}

/// The catalog of every registered tool (specs as JSON).
#[tauri::command]
fn list_tools(state: State<'_, AppState>) -> Result<Json, String> {
    serde_json::to_value(state.engine().list_tools()).map_err(|e| e.to_string())
}

/// Run one tool through the full guarded lifecycle and return its structured
/// `ToolResult` as JSON. Never throws — tool failures come back as `ok:false`.
#[tauri::command]
async fn invoke_tool(
    state: State<'_, AppState>,
    name: String,
    args: Json,
) -> Result<Json, String> {
    let engine = state.engine();
    let result = engine.invoke_tool(&name, args).await;
    serde_json::to_value(result).map_err(|e| e.to_string())
}

/// Drive a scripted (offline) goal loop and return `{ outcome, session }`.
///
/// `responses` is a JSON array of `CompletionResponse` objects (the mock model's
/// scripted turns) — used until a live model provider is wired in a later phase.
#[tauri::command]
async fn run_goal(
    state: State<'_, AppState>,
    goal: String,
    title: String,
    protocol: Option<String>,
    responses: Json,
) -> Result<Json, String> {
    let engine = state.engine();
    let proto = match protocol.as_deref() {
        Some("re_act_text") | Some("react") | Some("react_text") => Protocol::ReActText,
        _ => Protocol::NativeToolCall,
    };
    let resp: Vec<CompletionResponse> =
        serde_json::from_value(responses).map_err(|e| format!("invalid responses: {e}"))?;
    let (outcome, session) = engine
        .run_goal_scripted(&goal, &title, proto, resp)
        .await
        .map_err(|e| e.to_string())?;
    Ok(serde_json::json!({
        "outcome": outcome_to_json(&outcome),
        "session": serde_json::to_value(&session).map_err(|e| e.to_string())?,
    }))
}

/// Cancel any in-flight tool / loop work sharing this context.
#[tauri::command]
fn cancel(state: State<'_, AppState>) {
    state.engine().cancel();
}

/// One chat turn from the UI.
#[derive(serde::Deserialize)]
struct ChatMsg {
    role: String,
    content: String,
}

/// Plain multi-turn chat with the active model (no tools) — used by the 策划
/// (planning) screen's "和 AI 探讨" discussion. Returns the assistant's reply.
#[tauri::command]
async fn chat(app: tauri::AppHandle, messages: Vec<ChatMsg>) -> Result<String, String> {
    let path = providers_path(&app)?;
    let store = ProviderStore::open(&path).map_err(|e| e.to_string())?;
    let provider = store.build_active().map_err(|e| e.to_string())?;

    let msgs: Vec<Message> = messages
        .into_iter()
        .map(|m| match m.role.as_str() {
            "system" => Message::system(m.content),
            "assistant" => Message::assistant(m.content),
            _ => Message::user(m.content),
        })
        .collect();

    let req = CompletionRequest::new(msgs, Vec::new(), Protocol::ReActText);
    let resp = provider.complete(req).await.map_err(|e| e.to_string())?;
    Ok(resp.text)
}

/// A compact title for a discussion session, from its first user turn.
fn discuss_title(messages: &[ChatMsg]) -> String {
    let first = messages
        .iter()
        .find(|m| m.role != "system" && !m.content.trim().is_empty());
    match first {
        Some(m) => {
            let line: String = m.content.split_whitespace().collect::<Vec<_>>().join(" ");
            if line.chars().count() > 20 {
                let head: String = line.chars().take(20).collect();
                format!("{head}…")
            } else {
                line
            }
        }
        None => "探讨".to_string(),
    }
}

/// Streaming multi-turn chat with the active model — used by the 探讨 screen so
/// the reply types out live. Emits each text fragment on the `chat-delta` event.
///
/// The whole thread (user/assistant turns + the new reply) is persisted as a
/// `discuss` session so it shows up in the 会话 library and can be resumed.
/// Returns `{ text, session_id }`; pass `session_id` back on the next turn to
/// keep appending to the same thread.
#[tauri::command]
async fn chat_stream(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    messages: Vec<ChatMsg>,
    session_id: Option<String>,
) -> Result<Json, String> {
    let path = providers_path(&app)?;
    let store = ProviderStore::open(&path).map_err(|e| e.to_string())?;
    let provider = store.build_active().map_err(|e| e.to_string())?;
    let sessions = sessions_dir(&app, &state)?;

    let wire: Vec<Message> = messages
        .iter()
        .map(|m| match m.role.as_str() {
            "system" => Message::system(m.content.clone()),
            "assistant" => Message::assistant(m.content.clone()),
            _ => Message::user(m.content.clone()),
        })
        .collect();

    let req = CompletionRequest::new(wire, Vec::new(), Protocol::ReActText);

    let sink_app = app.clone();
    let on_delta = move |delta: &str| {
        let _ = sink_app.emit("chat-delta", serde_json::json!({ "delta": delta }));
    };
    let resp = provider
        .complete_streaming(req, &on_delta)
        .await
        .map_err(|e| e.to_string())?;

    // Persist the thread (user/assistant turns) + the new reply as a session.
    let sess_store = SessionStore::open(&sessions).map_err(|e| e.to_string())?;
    let mut session = Session::new(discuss_title(&messages));
    if let Some(id) = &session_id {
        session.id = SessionId::from_existing(id.clone());
    }
    for m in &messages {
        match m.role.as_str() {
            "system" => {}
            "assistant" => session.push(Message::assistant(m.content.clone())),
            _ => session.push(Message::user(m.content.clone())),
        }
    }
    if !resp.text.trim().is_empty() {
        session.push(Message::assistant(resp.text.clone()));
    }
    let saved_id = session.id.as_str().to_string();
    let _ = sess_store.save(&SessionRecord {
        session,
        kind: "discuss".to_string(),
        goal: None,
    });

    Ok(serde_json::json!({ "text": resp.text, "session_id": saved_id }))
}

/// List all persisted sessions (newest first) as lightweight summaries.
#[tauri::command]
fn sessions_list(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<Vec<SessionSummary>, String> {
    let store = SessionStore::open(sessions_dir(&app, &state)?).map_err(|e| e.to_string())?;
    store.list().map_err(|e| e.to_string())
}

/// Load one full session record (session + kind + goal) for resuming.
#[tauri::command]
fn session_get(app: tauri::AppHandle, state: State<'_, AppState>, id: String) -> Result<SessionRecord, String> {
    let store = SessionStore::open(sessions_dir(&app, &state)?).map_err(|e| e.to_string())?;
    store.get(&id).map_err(|e| e.to_string())
}

/// Delete a session; returns the updated list.
#[tauri::command]
fn session_delete(app: tauri::AppHandle, state: State<'_, AppState>, id: String) -> Result<Vec<SessionSummary>, String> {
    let store = SessionStore::open(sessions_dir(&app, &state)?).map_err(|e| e.to_string())?;
    store.delete(&id).map_err(|e| e.to_string())?;
    store.list().map_err(|e| e.to_string())
}

// ---- Story State Management ----

/// Load story state from workspace/story_state.json.
#[tauri::command]
fn story_state_load(state: State<'_, AppState>) -> Result<na_runtime::StoryState, String> {
    let engine = state.engine();
    let state_path = engine.ctx.jail.root().join("story_state.json");
    let mgr = na_runtime::StoryStateManager::open(&state_path).map_err(|e| e.to_string())?;
    Ok(mgr.state)
}

/// Save story state to workspace/story_state.json.
#[tauri::command]
fn story_state_save(
    state: State<'_, AppState>,
    story: na_runtime::StoryState,
) -> Result<(), String> {
    let engine = state.engine();
    let state_path = engine.ctx.jail.root().join("story_state.json");
    let mut mgr = na_runtime::StoryStateManager::open(&state_path).map_err(|e| e.to_string())?;
    mgr.state = story;
    mgr.save().map_err(|e| e.to_string())
}

/// Prepare context package for a given chapter (for preview/debugging).
#[tauri::command]
fn story_state_prepare_context(
    state: State<'_, AppState>,
    chapter_num: u32,
) -> Result<Json, String> {
    let engine = state.engine();
    let state_path = engine.ctx.jail.root().join("story_state.json");
    let mgr = na_runtime::StoryStateManager::open(&state_path).map_err(|e| e.to_string())?;
    let ctx_pkg = mgr.prepare_context(chapter_num);
    // Return as generic JSON since ContextPackage isn't Serialize
    serde_json::to_value(&ctx_pkg).map_err(|e| e.to_string())
}

// ---- 书库 / Multi-work management ----

/// List every work (newest first), with the active one flagged.
#[tauri::command]
fn works_list(state: State<'_, AppState>) -> Result<Vec<WorkSummary>, String> {
    Ok(state.works.lock().unwrap().list())
}

/// The active work's full metadata (or null if none).
#[tauri::command]
fn works_current(state: State<'_, AppState>) -> Result<Option<WorkMeta>, String> {
    Ok(state.works.lock().unwrap().active().cloned())
}

/// Create a new work and switch to it; rebuilds the engine. Returns the new work.
#[tauri::command]
fn works_create(
    state: State<'_, AppState>,
    title: String,
    blurb: Option<String>,
    genre: Option<String>,
    source_material: Option<String>,
) -> Result<WorkMeta, String> {
    let meta = {
        let mut works = state.works.lock().unwrap();
        works
            .create(
                title,
                blurb.unwrap_or_default(),
                genre.unwrap_or_default(),
                source_material.unwrap_or_default(),
            )
            .map_err(|e| e.to_string())?
    };
    rebuild_engine_to_active(&state)?;
    Ok(meta)
}

/// Switch the active work; rebuilds the engine to its workspace.
#[tauri::command]
fn works_open(state: State<'_, AppState>, id: String) -> Result<Vec<WorkSummary>, String> {
    {
        let mut works = state.works.lock().unwrap();
        works.set_active(&id).map_err(|e| e.to_string())?;
    }
    rebuild_engine_to_active(&state)?;
    Ok(state.works.lock().unwrap().list())
}

/// Rename / re-blurb / re-tag a work.
#[tauri::command]
fn works_update(
    state: State<'_, AppState>,
    id: String,
    title: Option<String>,
    blurb: Option<String>,
    genre: Option<String>,
    source_material: Option<String>,
) -> Result<WorkMeta, String> {
    let mut works = state.works.lock().unwrap();
    works
        .update(&id, title, blurb, genre, source_material)
        .map_err(|e| e.to_string())
}

/// Delete a work (optionally purging its files); rebuilds engine if active changed.
#[tauri::command]
fn works_delete(
    state: State<'_, AppState>,
    id: String,
    purge_files: Option<bool>,
) -> Result<Vec<WorkSummary>, String> {
    {
        let mut works = state.works.lock().unwrap();
        works
            .delete(&id, purge_files.unwrap_or(true))
            .map_err(|e| e.to_string())?;
    }
    // The active work may have changed; rebuild if there's still one.
    if state.works.lock().unwrap().active().is_some() {
        rebuild_engine_to_active(&state)?;
    }
    Ok(state.works.lock().unwrap().list())
}

// ---- 知识库 / Knowledge bases ----

/// List the active work's knowledge bases.
#[tauri::command]
fn knowledge_list_bases(state: State<'_, AppState>) -> Result<Vec<KnowledgeBaseMeta>, String> {
    let dir = active_knowledge_dir(&state)?;
    KnowledgeStore::open(&dir)
        .and_then(|s| s.list_bases())
        .map_err(|e| e.to_string())
}

/// Create a new knowledge base in the active work.
#[tauri::command]
fn knowledge_create_base(
    state: State<'_, AppState>,
    name: String,
    description: Option<String>,
) -> Result<KnowledgeBaseMeta, String> {
    let dir = active_knowledge_dir(&state)?;
    KnowledgeStore::open(&dir)
        .and_then(|s| s.create_base(name, description.unwrap_or_default()))
        .map_err(|e| e.to_string())
}

/// Delete a knowledge base.
#[tauri::command]
fn knowledge_delete_base(state: State<'_, AppState>, kb_id: String) -> Result<(), String> {
    let dir = active_knowledge_dir(&state)?;
    KnowledgeStore::open(&dir)
        .and_then(|s| s.delete_base(&kb_id))
        .map_err(|e| e.to_string())
}

/// Toggle whether a base participates in RAG retrieval.
#[tauri::command]
fn knowledge_set_active(
    state: State<'_, AppState>,
    kb_id: String,
    active: bool,
) -> Result<KnowledgeBaseMeta, String> {
    let dir = active_knowledge_dir(&state)?;
    KnowledgeStore::open(&dir)
        .and_then(|s| s.set_base_active(&kb_id, active))
        .map_err(|e| e.to_string())
}

/// Rename / re-describe a base.
#[tauri::command]
fn knowledge_update_base(
    state: State<'_, AppState>,
    kb_id: String,
    name: Option<String>,
    description: Option<String>,
) -> Result<KnowledgeBaseMeta, String> {
    let dir = active_knowledge_dir(&state)?;
    KnowledgeStore::open(&dir)
        .and_then(|s| s.update_base(&kb_id, name, description))
        .map_err(|e| e.to_string())
}

/// List all entries in a base (full content, newest first).
#[tauri::command]
fn knowledge_list_entries(
    state: State<'_, AppState>,
    kb_id: String,
) -> Result<Vec<KnowledgeEntry>, String> {
    let dir = active_knowledge_dir(&state)?;
    let store = KnowledgeStore::open(&dir).map_err(|e| e.to_string())?;
    let kb = store.open_base(&kb_id).map_err(|e| e.to_string())?;
    Ok(kb.entries())
}

/// Add an entry to a base. `kind` is one of the KnowledgeKind snake_case names.
#[tauri::command]
fn knowledge_add_entry(
    state: State<'_, AppState>,
    kb_id: String,
    kind: String,
    title: String,
    content: String,
    source: Option<String>,
    tags: Option<Vec<String>>,
) -> Result<String, String> {
    let dir = active_knowledge_dir(&state)?;
    let store = KnowledgeStore::open(&dir).map_err(|e| e.to_string())?;
    let mut kb = store.open_base(&kb_id).map_err(|e| e.to_string())?;
    let k = parse_kind(&kind);
    kb.add(
        k,
        title,
        content,
        source.unwrap_or_else(|| "user".to_string()),
        tags.unwrap_or_default(),
    )
    .map_err(|e| e.to_string())
}

/// Remove an entry from a base.
#[tauri::command]
fn knowledge_delete_entry(
    state: State<'_, AppState>,
    kb_id: String,
    entry_id: String,
) -> Result<(), String> {
    let dir = active_knowledge_dir(&state)?;
    let store = KnowledgeStore::open(&dir).map_err(|e| e.to_string())?;
    let mut kb = store.open_base(&kb_id).map_err(|e| e.to_string())?;
    kb.remove(&entry_id).map_err(|e| e.to_string())
}

/// Search across all *active* knowledge bases (RAG preview).
#[tauri::command]
fn knowledge_search(
    state: State<'_, AppState>,
    query: String,
    k: Option<usize>,
) -> Result<Vec<KnowledgeHit>, String> {
    let dir = active_knowledge_dir(&state)?;
    KnowledgeStore::open(&dir)
        .and_then(|s| s.search_active(&query, k.unwrap_or(8)))
        .map_err(|e| e.to_string())
}

/// Map a snake_case kind name to the enum (defaults to Other).
fn parse_kind(s: &str) -> KnowledgeKind {
    match s {
        "character" => KnowledgeKind::Character,
        "location" => KnowledgeKind::Location,
        "worldbuilding" => KnowledgeKind::Worldbuilding,
        "event" => KnowledgeKind::Event,
        "item" => KnowledgeKind::Item,
        "term" => KnowledgeKind::Term,
        "lore" => KnowledgeKind::Lore,
        _ => KnowledgeKind::Other,
    }
}

/// Use the active model + its web-fetch tools to auto-fill a knowledge base from
/// the work's source material. The agent runs a goal loop where it can:
/// 1. Use `http_get` / web tools to fetch canon material
/// 2. Call a dynamically-registered `knowledge_save` tool to write entries
///
/// Streams progress on `agent-step`. Returns the run outcome + entry count.
#[tauri::command]
async fn knowledge_fill_web(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    kb_id: String,
    topic: String,
) -> Result<Json, String> {
    let path = providers_path(&app)?;
    let store = ProviderStore::open(&path).map_err(|e| e.to_string())?;
    let agent_protocol = store
        .active()
        .map(|(cfg, _model)| cfg.agent_protocol())
        .unwrap_or(Protocol::NativeToolCall);
    let provider = store.build_active().map_err(|e| e.to_string())?;
    let engine = state.engine();
    let knowledge_dir = active_knowledge_dir(&state)?;

    // Inject a custom `knowledge_save` tool into this run's engine clone.
    // We build a temporary engine with the extra tool so the agent can write
    // entries directly during the loop.
    let mut registry = engine.registry.clone();
    let kb_tool = KnowledgeSaveTool {
        knowledge_dir: knowledge_dir.clone(),
        kb_id: kb_id.clone(),
    };
    let _ = registry.register(Arc::new(kb_tool));

    let goal = format!(
        "你是一名资料整理专家。请研究「{}」这部作品，使用 http_get 工具联网获取相关设定资料\
        （维基、百科、设定集等），然后调用 knowledge_save 工具将整理好的设定条目保存到知识库。\
        \n\n目标知识库 ID: {}\
        \n\n要求：\n\
        1. 使用 http_get 获取至少 2-3 个相关网页\n\
        2. 提取核心人物、世界规则、重要地点、关键事件、专有术语\n\
        3. 每条设定调用一次 knowledge_save，kind 从 character/location/worldbuilding/event/item/term/lore 中选择\n\
        4. 目标产出 8-20 条结构化设定条目\n\
        5. 最后总结共保存了多少条目",
        topic, kb_id
    );

    let mut session = Session::new("联网填充知识库");
    session.push(Message::user(goal));

    // Stream each step to the UI.
    let mut hooks = LoopHookRegistry::new();
    hooks.register(Arc::new(TauriLoopHook { app: app.clone() }));

    let outcome = GoalLoop::with_protocol(agent_protocol)
        .loop_hooks(Arc::new(hooks))
        .run(
            &format!("研究「{}」并填充知识库", topic),
            &mut session,
            &provider,
            &registry,
            &engine.ctx,
        )
        .await
        .map_err(|e| e.to_string())?;

    // Count how many entries were saved by checking the final session for
    // successful knowledge_save tool results.
    let saved_count = session
        .history()
        .iter()
        .filter(|m| {
            m.content.contains("knowledge_save") && m.content.contains("已保存设定条目")
        })
        .count();

    // Reload the bases list so the UI sees the updated entry_count.
    let kstore = KnowledgeStore::open(&knowledge_dir).map_err(|e| e.to_string())?;
    let _ = kstore.open_base(&kb_id); // trigger meta save

    Ok(serde_json::json!({
        "outcome": outcome_to_json(&outcome),
        "added": saved_count,
        "session": serde_json::to_value(&session).map_err(|e| e.to_string())?,
    }))
}

/// A dynamically-registered tool that writes to a specific knowledge base.
/// Lives only for the duration of a single `knowledge_fill_web` run.
struct KnowledgeSaveTool {
    knowledge_dir: PathBuf,
    kb_id: String,
}

impl na_tools::Tool for KnowledgeSaveTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "knowledge_save".to_string(),
            description: "将一条设定资料保存到当前知识库。kind 可选值: character, location, \
                worldbuilding, event, item, term, lore, other。"
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "kind": {
                        "type": "string",
                        "enum": ["character", "location", "worldbuilding", "event", "item", "term", "lore", "other"],
                        "description": "条目类型"
                    },
                    "title": { "type": "string", "description": "标题" },
                    "content": { "type": "string", "description": "详细设定" },
                    "tags": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "标签（可选）"
                    },
                    "source": { "type": "string", "description": "来源 URL 或描述（可选）" }
                },
                "required": ["kind", "title", "content"]
            }),
            capabilities: vec![Capability::WriteMemory],
            mutating: true,
            concurrency: Default::default(),
        }
    }

    fn execute<'a>(
        &'a self,
        args: serde_json::Value,
        _ctx: &'a na_tools::ToolContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = na_common::Result<na_tools::ToolResult>> + Send + 'a>>
    {
        Box::pin(async move {
            let kind_str = args
                .get("kind")
                .and_then(|v| v.as_str())
                .ok_or_else(|| CoreError::invalid_input("missing kind"))?;
            let title = args
                .get("title")
                .and_then(|v| v.as_str())
                .ok_or_else(|| CoreError::invalid_input("missing title"))?;
            let content = args
                .get("content")
                .and_then(|v| v.as_str())
                .ok_or_else(|| CoreError::invalid_input("missing content"))?;
            let tags: Vec<String> = args
                .get("tags")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();
            let source = args
                .get("source")
                .and_then(|v| v.as_str())
                .unwrap_or("web");

            let store = KnowledgeStore::open(&self.knowledge_dir)
                .map_err(|e| CoreError::internal(format!("opening KB store: {e}")))?;
            let mut kb = store
                .open_base(&self.kb_id)
                .map_err(|e| CoreError::internal(format!("opening KB: {e}")))?;
            let kind = parse_kind(kind_str);
            let entry_id = kb
                .add(kind, title, content, source, tags)
                .map_err(|e| CoreError::internal(format!("saving entry: {e}")))?;

            Ok(na_tools::ToolResult {
                ok: true,
                content: format!("已保存设定条目「{title}」"),
                summary: Some(format!("saved: {title}")),
                data: serde_json::json!({ "entry_id": entry_id }),
                metadata: ResultMeta {
                    bytes: title.len() + content.len(),
                    truncated: false,
                    was_binary: false,
                    redactions: 0,
                    untrusted: false,
                    duration_ms: 0,
                },
            })
        })
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            // The whole library lives under the OS app-data dir. Each work gets
            // its own isolated workspace under `works/<id>/workspace`. On first
            // run we adopt any pre-existing top-level `workspace/` (and
            // `sessions/`) from older builds as the "默认作品" so manuscripts
            // survive the upgrade without a migration step.
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;

            let mut works = WorkStore::open(&data_dir)?;
            let legacy_ws = data_dir.join("workspace");
            let legacy_sessions = data_dir.join("sessions");
            works.adopt_legacy(&legacy_ws, &legacy_sessions)?;

            // If the library is still empty (fresh install), create a starter work.
            if works.active().is_none() {
                works.create("我的第一部作品", "", "", "")?;
            }

            // Build the engine pointed at the active work's workspace.
            let workspace_dir = works
                .active()
                .expect("an active work exists after setup")
                .workspace_dir
                .clone();
            std::fs::create_dir_all(&workspace_dir)?;
            // Ensure the default writer.md exists so the AI always has
            // explicit instructions to save chapters via write_file.
            ensure_default_writer_md(&workspace_dir);
            let engine = Engine::new(&workspace_dir)?;

            app.manage(AppState {
                engine: RwLock::new(Arc::new(engine)),
                works: Mutex::new(works),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            ping,
            list_tools,
            invoke_tool,
            run_goal,
            cancel,
            providers_get,
            providers_save,
            providers_delete,
            providers_set_active,
            provider_test,
            run_goal_live,
            chat,
            chat_stream,
            sessions_list,
            session_get,
            session_delete,
            story_state_load,
            story_state_save,
            story_state_prepare_context,
            works_list,
            works_current,
            works_create,
            works_open,
            works_update,
            works_delete,
            knowledge_list_bases,
            knowledge_create_base,
            knowledge_delete_base,
            knowledge_set_active,
            knowledge_update_base,
            knowledge_list_entries,
            knowledge_add_entry,
            knowledge_delete_entry,
            knowledge_search,
            knowledge_fill_web
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
