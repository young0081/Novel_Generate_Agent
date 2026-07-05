// IdeAiPanel — 增强版 IDE AI 助手面板
// 双模式：「对话」（流式聊天）| 「运笔」（完整 Agent loop + 工具可视化）
// 头部集成 ModelSelector，运笔模式下可停止，并有 ReasoningBlock / ToolCallCard 展示

import { useCallback, useEffect, useRef, useState } from "react";
import { chatStream, runGoalLive, type ChatMessage } from "../../lib/studio";
import {
  IconBrush,
  IconUser,
  IconProviders,
  IconRefresh,
  IconStop,
  IconSend,
  IconAgentMode,
  IconThought,
} from "../icons";
import { Spinner } from "../Spinner";
import { useToast } from "../Toast";
import { getProviders } from "../../lib/providers";
import ModelSelector from "../ModelSelector";
import ReasoningBlock from "../agent/ReasoningBlock";
import { ToolCallCard } from "../agent/ToolCallCard";
import WorkStatus from "../agent/WorkStatus";
import WorkflowSteps from "../agent/WorkflowSteps";
import {
  upsertStep,
  derivePhase,
  workflowView,
  type RunStep,
  type AgentPhase,
  IDE_STAGES,
} from "../../lib/agentRun";

// ── 类型 ────────────────────────────────────────────────────────

interface IdeAiPanelProps {
  filePath: string | null;
  getFileContent: () => string;
  onSettingsOpen?: () => void;
  /** Called after agent run with the list of files the agent wrote. */
  onFilesModified?: (paths: string[]) => void;
  /** Called when user clicks "插入到编辑器" on an AI reply. */
  onInsert?: (text: string) => void;
}

type PanelMode = "chat" | "agent";

interface Turn {
  role: "user" | "assistant";
  content: string;
  streaming?: boolean;
  /** 流式推理文本（thinking 阶段增量） */
  reasoning?: string;
}

// ── 快捷指令芯片 ─────────────────────────────────────────────────

const CHAT_CHIPS = [
  { label: "续写这一段", icon: "✍️" },
  { label: "改得更有张力", icon: "⚡" },
  { label: "检查前后矛盾", icon: "🔍" },
  { label: "建议一个转折", icon: "🌀" },
];

const AGENT_CHIPS = [
  { label: "续写当前章节并保存", icon: "✍️" },
  { label: "润色语言，提升文学性", icon: "✨" },
  { label: "检查角色一致性，修正错误", icon: "🎭" },
  { label: "添加伏笔，与开头呼应", icon: "🧵" },
];

// ── 主组件 ───────────────────────────────────────────────────────

export default function IdeAiPanel({
  filePath,
  getFileContent,
  onSettingsOpen,
  onFilesModified,
  onInsert,
}: IdeAiPanelProps) {
  const toast = useToast();
  const [mode, setMode] = useState<PanelMode>("chat");
  const [hasProvider, setHasProvider] = useState<boolean | null>(null);

  // 对话模式状态
  const [turns, setTurns] = useState<Turn[]>([]);
  const [draft, setDraft] = useState("");
  const [sending, setSending] = useState(false);
  const [sessionId, setSessionId] = useState<string | null>(null);

  // 运笔模式状态
  const [agentGoal, setAgentGoal] = useState("");
  const [agentSteps, setAgentSteps] = useState<RunStep[]>([]);
  const [agentRunning, setAgentRunning] = useState(false);
  const [agentDone, setAgentDone] = useState(false);
  const [agentError, setAgentError] = useState<string | null>(null);
  const [agentAnswer, setAgentAnswer] = useState<string | null>(null);
  const stepKeyRef = useRef(0);
  const cancelAgentRef = useRef<(() => void) | null>(null);

  const bottomRef = useRef<HTMLDivElement>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  // 检查是否有供应商
  useEffect(() => {
    getProviders()
      .then((p) => setHasProvider(p.providers.length > 0))
      .catch(() => setHasProvider(false));
  }, []);

  // 自动滚底
  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [turns, agentSteps]);

  // ── 对话模式 send ────────────────────────────────────────────

  const sendChat = useCallback(async () => {
    const text = draft.trim();
    if (!text || sending) return;

    const fileContent = getFileContent();
    const contextNote = filePath
      ? `\n\n---\n【当前文件：${filePath}】\n${fileContent.slice(0, 4000)}${fileContent.length > 4000 ? "\n…（已截断）" : ""}`
      : "";

    const userMsg = text + (turns.length === 0 ? contextNote : "");

    setDraft("");
    setSending(true);
    setTurns((prev) => [...prev, { role: "user", content: text }]);

    const history: ChatMessage[] = [
      {
        role: "system",
        content:
          "你是专业的小说创作助手，正在协助用户在 IDE 环境中编辑和改进章节内容。" +
          (filePath ? `\n当前编辑的文件：${filePath}` : ""),
      },
      ...turns.map((t) => ({ role: t.role as "user" | "assistant", content: t.content })),
      { role: "user", content: userMsg },
    ];

    let accumulated = "";
    let reasoningAcc = "";
    setTurns((prev) => [...prev, { role: "assistant", content: "", streaming: true }]);

    try {
      const result = await chatStream(
        history,
        (chunk: string) => {
          // chunk 以 "\x00" 前缀标识推理增量（若后端支持推理流）
          if (chunk.startsWith("\x00")) {
            reasoningAcc += chunk.slice(1);
            setTurns((prev) => {
              const next = [...prev];
              const last = next[next.length - 1];
              if (last?.role === "assistant") {
                next[next.length - 1] = { ...last, reasoning: reasoningAcc, streaming: true };
              }
              return next;
            });
          } else {
            accumulated += chunk;
            setTurns((prev) => {
              const next = [...prev];
              const last = next[next.length - 1];
              if (last?.role === "assistant") {
                next[next.length - 1] = { ...last, content: accumulated, streaming: true };
              }
              return next;
            });
          }
        },
        sessionId ?? undefined,
      );
      setSessionId(result.sessionId);
      setTurns((prev) => {
        const next = [...prev];
        const last = next[next.length - 1];
        if (last?.role === "assistant") {
          next[next.length - 1] = {
            ...last,
            content: result.text || accumulated,
            reasoning: reasoningAcc || undefined,
            streaming: false,
          };
        }
        return next;
      });
    } catch {
      setTurns((prev) => prev.filter((t) => !t.streaming));
      toast.err("AI 回复失败，请重试");
    } finally {
      setSending(false);
    }
  }, [draft, sending, turns, filePath, getFileContent, sessionId, toast]);

  // ── 运笔模式 run ─────────────────────────────────────────────

  const runAgent = useCallback(async (goal?: string) => {
    const targetGoal = (goal ?? agentGoal).trim();
    if (!targetGoal || agentRunning) return;

    if (goal) setAgentGoal(goal);

    // 构建含文件上下文的 goal
    const fileContent = getFileContent();
    const fullGoal = filePath
      ? `${targetGoal}\n\n【当前文件：${filePath}】\n${fileContent.slice(0, 3000)}${fileContent.length > 3000 ? "\n…（已截断）" : ""}`
      : targetGoal;

    const chapterTitle = filePath
      ? filePath.replace(/.*\//, "").replace(/\.\w+$/, "")
      : "IDE任务";

    setAgentRunning(true);
    setAgentDone(false);
    setAgentError(null);
    setAgentAnswer(null);
    setAgentSteps([]);

    let cancelled = false;
    cancelAgentRef.current = () => { cancelled = true; };

    // Collect file paths written by the agent so we can refresh open editors.
    const writtenPaths = new Set<string>();

    try {
      const result = await runGoalLive(
        fullGoal,
        chapterTitle,
        (event) => {
          if (cancelled) return;
          // Track write_file / append_file tool calls so we can tell the
          // editor which files changed on disk.
          if (event.phase === "model" && event.tool_calls) {
            for (const tc of event.tool_calls) {
              const n = tc.name.toLowerCase();
              if (n.includes("write") || n.includes("append") || n.includes("save")) {
                const args = tc.args as Record<string, unknown> | null;
                const p = args?.path ?? args?.file_path ?? args?.file;
                if (typeof p === "string" && p.trim()) {
                  writtenPaths.add(p.trim());
                }
              }
            }
          }
          setAgentSteps((prev) => upsertStep(prev, event, () => (stepKeyRef.current += 1)));
        },
      );
      if (!cancelled) {
        setAgentAnswer(result.outcome.final_answer ?? null);
        setAgentDone(true);
        if (writtenPaths.size > 0) {
          onFilesModified?.(Array.from(writtenPaths));
        }
      }
    } catch (e: unknown) {
      if (!cancelled) {
        setAgentError(e instanceof Error ? e.message : String(e));
      }
    } finally {
      setAgentRunning(false);
      cancelAgentRef.current = null;
    }
  }, [agentGoal, agentRunning, filePath, getFileContent, onFilesModified]);

  const stopAgent = useCallback(() => {
    cancelAgentRef.current?.();
    setAgentRunning(false);
  }, []);

  const clearAgent = useCallback(() => {
    setAgentSteps([]);
    setAgentGoal("");
    setAgentDone(false);
    setAgentError(null);
    setAgentAnswer(null);
  }, []);

  const clearChat = useCallback(() => {
    setTurns([]);
    setSessionId(null);
  }, []);

  // ── 键盘处理 ─────────────────────────────────────────────────

  const handleChatKey = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        sendChat();
      }
    },
    [sendChat],
  );

  const handleAgentKey = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        runAgent();
      }
    },
    [runAgent],
  );

  // ── 无供应商提示 ─────────────────────────────────────────────

  if (hasProvider === false) {
    return (
      <div className="ide-ai ide-ai--no-provider">
        <div className="ide-ai__callout">
          <div className="ide-ai__callout-icon">
            <IconProviders size={22} />
          </div>
          <div className="ide-ai__callout-body">
            <strong>尚未配置 AI 供应商</strong>
            <p>配置后可在此处与 AI 对话或让 AI 直接修改文件。</p>
          </div>
          {onSettingsOpen && (
            <button className="ide-ai__callout-btn" onClick={onSettingsOpen}>
              去配置
            </button>
          )}
        </div>
      </div>
    );
  }

  // ── 推导 Agent 状态 ──────────────────────────────────────────

  const agentPhase: AgentPhase = derivePhase({
    running: agentRunning,
    steps: agentSteps,
    finished: agentDone || !!agentError,
    success: agentDone && !agentError,
    errored: !!agentError,
  });
  const { current: wfCurrent, state: wfState } = workflowView(agentPhase);

  // ── 渲染 ─────────────────────────────────────────────────────

  return (
    <div className="ide-ai">
      {/* ── 顶部工具栏 ── */}
      <div className="ide-ai__header">
        {/* 模式切换 */}
        <div className="ide-ai__mode-tabs">
          <button
            className={`ide-ai__mode-tab${mode === "chat" ? " ide-ai__mode-tab--active" : ""}`}
            onClick={() => setMode("chat")}
            title="对话模式"
          >
            <IconBrush size={12} />
            对话
          </button>
          <button
            className={`ide-ai__mode-tab${mode === "agent" ? " ide-ai__mode-tab--active" : ""}`}
            onClick={() => setMode("agent")}
            title="运笔模式（Agent 自动修改文件）"
          >
            <IconAgentMode size={12} />
            运笔
          </button>
        </div>

        {/* 右侧操作区 */}
        <div className="ide-ai__header-right">
          {mode === "chat" && turns.length > 0 && (
            <button className="ide-ai__icon-btn" onClick={clearChat} title="清空对话">
              <IconRefresh size={12} />
            </button>
          )}
          {mode === "agent" && (agentSteps.length > 0 || agentDone || agentError) && !agentRunning && (
            <button className="ide-ai__icon-btn" onClick={clearAgent} title="清空记录">
              <IconRefresh size={12} />
            </button>
          )}
          {mode === "agent" && agentRunning && (
            <button className="ide-ai__stop-btn" onClick={stopAgent} title="停止运笔">
              <IconStop size={12} />
              停止
            </button>
          )}
        </div>
      </div>

      {/* ── 模型选择器 ── */}
      <div className="ide-ai__model-bar">
        <ModelSelector
          size="sm"
          className="ide-ai__model-sel"
          onSettingsOpen={onSettingsOpen}
        />
      </div>

      {/* ── 内容区 ── */}
      <div className="ide-ai__body">
        {mode === "chat" ? (
          <ChatPane
            turns={turns}
            sending={sending}
            filePath={filePath}
            onChip={(chip) => { setDraft(chip); textareaRef.current?.focus(); }}
            onInsert={onInsert}
            bottomRef={bottomRef}
          />
        ) : (
          <AgentPane
            steps={agentSteps}
            running={agentRunning}
            done={agentDone}
            error={agentError}
            answer={agentAnswer}
            phase={agentPhase}
            wfCurrent={wfCurrent}
            wfState={wfState}
            onChip={runAgent}
            bottomRef={bottomRef}
          />
        )}
      </div>

      {/* ── 输入区 ── */}
      <div className="ide-ai__footer">
        {mode === "chat" ? (
          <div className="ide-ai__input-row">
            <textarea
              ref={textareaRef}
              className="ide-ai__input"
              value={draft}
              onChange={(e) => setDraft(e.target.value)}
              onKeyDown={handleChatKey}
              placeholder="问 AI…（Enter 发送，Shift+Enter 换行）"
              rows={2}
              disabled={sending}
            />
            <button
              className="ide-ai__send-btn"
              onClick={sendChat}
              disabled={!draft.trim() || sending}
              title="发送"
            >
              {sending ? <Spinner size={13} /> : <IconSend size={13} />}
            </button>
          </div>
        ) : (
          <div className="ide-ai__agent-input-row">
            <textarea
              className="ide-ai__input"
              value={agentGoal}
              onChange={(e) => setAgentGoal(e.target.value)}
              onKeyDown={handleAgentKey}
              placeholder="描述任务，AI 将自动编辑文件…"
              rows={2}
              disabled={agentRunning}
            />
            <button
              className="ide-ai__send-btn ide-ai__send-btn--agent"
              onClick={() => runAgent()}
              disabled={!agentGoal.trim() || agentRunning}
              title="开始运笔"
            >
              {agentRunning ? <Spinner size={13} /> : <IconAgentMode size={13} />}
            </button>
          </div>
        )}
        {filePath && (
          <div className="ide-ai__file-hint">{filePath}</div>
        )}
      </div>
    </div>
  );
}

// ── 对话面板 ────────────────────────────────────────────────────

interface ChatPaneProps {
  turns: Turn[];
  sending: boolean;
  filePath: string | null;
  onChip: (text: string) => void;
  onInsert?: (text: string) => void;
  bottomRef: React.RefObject<HTMLDivElement | null>;
}

function ChatPane({ turns, sending, filePath, onChip, onInsert, bottomRef }: ChatPaneProps) {
  return (
    <div className="ide-ai__messages">
      {turns.length === 0 && (
        <div className="ide-ai__welcome">
          <div className="ide-ai__welcome-label">快捷指令</div>
          <div className="ide-ai__chips">
            {CHAT_CHIPS.map((c) => (
              <button key={c.label} className="ide-ai__chip" onClick={() => onChip(c.label)}>
                <span>{c.icon}</span>
                {c.label}
              </button>
            ))}
          </div>
          {filePath && (
            <p className="ide-ai__file-note">当前：{filePath.replace(/.*\//, "")}</p>
          )}
        </div>
      )}

      {turns.map((turn, i) => (
        <div key={i} className={`ide-ai__turn ide-ai__turn--${turn.role}`}>
          <span className="ide-ai__avatar">
            {turn.role === "user" ? <IconUser size={12} /> : <IconBrush size={12} />}
          </span>
          <div className="ide-ai__bubble-wrap">
            {turn.role === "assistant" && turn.reasoning && (
              <div className="ide-ai__reasoning">
                <ReasoningBlock
                  text={turn.reasoning}
                  active={!!turn.streaming && !turn.content}
                />
              </div>
            )}
            <div className="ide-ai__bubble">
              {turn.content
                ? turn.content
                : turn.streaming
                ? <span className="ide-ai__caret" aria-hidden>▍</span>
                : null}
            </div>
            {/* P0-2: 插入按钮 — 仅 assistant 回复完毕后显示 */}
            {turn.role === "assistant" && !turn.streaming && turn.content && onInsert && (
              <button
                className="ide-ai__insert-btn"
                onClick={() => onInsert(turn.content)}
                title="在编辑器光标处插入此回复"
              >
                ↓ 插入到编辑器
              </button>
            )}
          </div>
        </div>
      ))}

      {sending && turns[turns.length - 1]?.role !== "assistant" && (
        <div className="ide-ai__thinking">
          <IconThought size={13} />
          <span>正在思考…</span>
        </div>
      )}

      <div ref={bottomRef} />
    </div>
  );
}

// ── 运笔面板 ────────────────────────────────────────────────────

interface AgentPaneProps {
  steps: RunStep[];
  running: boolean;
  done: boolean;
  error: string | null;
  answer: string | null;
  phase: AgentPhase;
  wfCurrent: number;
  wfState: "idle" | "running" | "done" | "stopped" | "error";
  onChip: (goal: string) => void;
  bottomRef: React.RefObject<HTMLDivElement | null>;
}

function AgentPane({
  steps,
  running,
  done,
  error,
  answer,
  phase,
  wfCurrent,
  wfState,
  onChip,
  bottomRef,
}: AgentPaneProps) {
  const idle = !running && !done && !error && steps.length === 0;

  return (
    <div className="ide-ai__agent-body">
      {idle ? (
        /* 空状态：快捷任务芯片 */
        <div className="ide-ai__welcome">
          <div className="ide-ai__welcome-label">常用任务</div>
          <div className="ide-ai__chips ide-ai__chips--col">
            {AGENT_CHIPS.map((c) => (
              <button
                key={c.label}
                className="ide-ai__chip"
                onClick={() => onChip(c.label)}
              >
                <span>{c.icon}</span>
                {c.label}
              </button>
            ))}
          </div>
          <p className="ide-ai__agent-hint">
            运笔模式会调用工具直接修改文件，请确保已保存重要内容。
          </p>
        </div>
      ) : (
        <>
          {/* 工作流阶段轨 */}
          <div className="ide-ai__workflow">
            <WorkflowSteps stages={IDE_STAGES} current={wfCurrent} state={wfState} />
          </div>

          {/* 状态条 */}
          <div className="ide-ai__workstatus">
            <WorkStatus
              phase={phase}
              step={steps.length}
              toolCount={steps.reduce((n, s) => n + (s.toolCalls?.length ?? 0), 0)}
            />
          </div>

          {/* 步骤流水线 */}
          <div className="ide-ai__steps">
            {steps.map((step) => (
              <div key={step.key} className="ide-ai__step">
                {/* 推理块 */}
                {step.text && (
                  <ReasoningBlock
                    text={step.text}
                    active={running && step.key === steps[steps.length - 1]?.key}
                  />
                )}
                {/* 工具调用卡片 */}
                {step.toolCalls?.map((tc, ti) => (
                  <ToolCallCard
                    key={ti}
                    name={tc.name}
                    args={tc.args}
                  />
                ))}
              </div>
            ))}
          </div>

          {/* 完成答复 */}
          {done && answer && (
            <div className="ide-ai__agent-answer">
              <div className="ide-ai__agent-answer-label">✓ 运笔完成</div>
              <div className="ide-ai__agent-answer-text">{answer}</div>
            </div>
          )}

          {/* 错误提示 */}
          {error && (
            <div className="ide-ai__agent-error">
              <strong>运笔中断：</strong>{error}
            </div>
          )}
        </>
      )}

      <div ref={bottomRef} />
    </div>
  );
}
