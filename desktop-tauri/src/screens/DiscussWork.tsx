// 探讨 — Full-screen immersive discussion with AI.
// Elevated to a standalone work mode (4th tab alongside Planning/Studio/Revision).
// Context anchor at top, conversation stream, input at bottom, quick-start topics.

import { useCallback, useEffect, useState } from "react";
import { Spinner } from "../components/Spinner";
import {
  IconChat,
  IconRefresh,
  IconUser,
  IconCompass,
  IconStar,
  IconProviders,
  IconBrush,
  IconCheck,
  IconArrowRight,
} from "../components/icons";
import { describeError } from "../lib/core";
import { useToast } from "../components/Toast";
import { chatStream, type ChatMessage } from "../lib/studio";
import { getProviders } from "../lib/providers";

interface DiscussWorkProps {
  onOpenSettings: () => void;
  initialSessionId?: string; // From SessionsDrawer "resume"
}

interface Turn {
  role: "user" | "assistant";
  content: string;
}

const DISCUSS_SYSTEM = `你是「墨·创作」的策划顾问，专长于同人小说的剧情设计、人物塑造与冲突构建。
用户会与你探讨他们的创意构思，请：
1. 深入追问，挖掘情感张力与戏剧冲突
2. 提出多角度的可能性，激发灵感
3. 指出潜在的叙事陷阱或人物塑造问题
4. 保持对话轻松，像朋友间的脑暴，不要过于说教

对话风格：亲切、启发性、具体而非空泛。`;

const QUICK_TOPICS = [
  { label: "人物深化", prompt: "帮我深化一下主角的性格与成长弧线" },
  { label: "情节推演", prompt: "我想探讨一下接下来的剧情走向，有哪些可能性？" },
  { label: "冲突设计", prompt: "这个故事的核心冲突应该如何设计？" },
  { label: "伏笔布局", prompt: "我想在前期埋下哪些伏笔比较合适？" },
];

export default function DiscussWork({ onOpenSettings, initialSessionId }: DiscussWorkProps) {
  const toast = useToast();
  const [concept, setConcept] = useState("");
  const [turns, setTurns] = useState<Turn[]>([]);
  const [draft, setDraft] = useState("");
  const [sending, setSending] = useState(false);
  const [streamingText, setStreamingText] = useState("");
  const [activeModel, setActiveModel] = useState<string | null>(null);
  const [loadingModel, setLoadingModel] = useState(true);
  const [sessionId, setSessionId] = useState<string | null>(initialSessionId ?? null);
  const [showSavedHint, setShowSavedHint] = useState(false);

  // Load active model on mount
  useEffect(() => {
    const load = async () => {
      setLoadingModel(true);
      try {
        const p = await getProviders();
        const active = p.providers.find((x) => x.id === p.active_provider);
        setActiveModel(active ? `${active.name}` : null);
      } catch {
        setActiveModel(null);
      } finally {
        setLoadingModel(false);
      }
    };
    void load();
  }, []);

  const sendMessage = useCallback(
    async (text?: string) => {
      const msg = (text || draft).trim();
      if (!msg) return;
      if (!activeModel) {
        toast.err("请先配置一个模型供应商");
        return;
      }

      const userTurn: Turn = { role: "user", content: msg };
      setTurns((prev) => [...prev, userTurn]);
      setDraft("");
      setSending(true);
      setStreamingText("");

      const history: ChatMessage[] = [
        { role: "system", content: DISCUSS_SYSTEM },
        ...turns.map((t) => ({ role: t.role, content: t.content })),
        { role: "user", content: msg },
      ];

      try {
        let accumulated = "";
        const result = await chatStream(
          history,
          (chunk: string) => {
            accumulated += chunk;
            setStreamingText(accumulated);
          },
          sessionId ?? undefined,
        );
        setSessionId(result.sessionId);
        setTurns((prev) => [
          ...prev,
          { role: "assistant", content: result.text },
        ]);
        setStreamingText("");
        setSending(false);

        // Show saved hint briefly
        setShowSavedHint(true);
        setTimeout(() => setShowSavedHint(false), 2500);
      } catch (e) {
        toast.err(`对话失败：${describeError(e)}`);
        setSending(false);
        setStreamingText("");
      }
    },
    [draft, activeModel, turns, toast, sessionId],
  );

  const handleQuickStart = useCallback(
    (prompt: string) => {
      if (sending) return;
      void sendMessage(prompt);
    },
    [sending, sendMessage],
  );

  const clearConversation = useCallback(() => {
    setTurns([]);
    setStreamingText("");
    setSessionId(null);
    toast.ok("已清空对话");
  }, [toast]);

  const modelDisplay = activeModel ? activeModel : "未配置";

  return (
    <div className="discuss">
      {/* Context anchor */}
      <div className="discuss__anchor">
        <div className="discuss__anchor-inner">
          <div className="discuss__anchor-row">
            <div className="discuss__anchor-label">
              <IconStar size={12} />
              当前构思
            </div>
            <div className="discuss__anchor-meta">
              {loadingModel ? (
                <div className="discuss__model-loading">
                  <Spinner size={11} />
                  加载中…
                </div>
              ) : activeModel ? (
                <div className="discuss__model-active">
                  <IconProviders size={11} />
                  {modelDisplay}
                </div>
              ) : (
                <button className="link-btn" onClick={onOpenSettings}>
                  <IconProviders size={11} />
                  配置模型
                </button>
              )}
              {showSavedHint && (
                <div className="discuss__saved-hint">
                  <IconCheck size={10} />
                  已自动存档
                </div>
              )}
            </div>
          </div>
          <textarea
            className="discuss__concept-input"
            value={concept}
            onChange={(e) => setConcept(e.target.value)}
            placeholder="简述你的故事构思，作为对话的锚点…"
            rows={2}
            spellCheck={false}
          />
          {turns.length > 0 && (
            <div className="discuss__anchor-actions">
              <button
                className="btn btn--ghost btn--sm"
                onClick={clearConversation}
                disabled={sending}
              >
                <IconRefresh size={13} />
                清空对话
              </button>
            </div>
          )}
        </div>
      </div>

      {/* Conversation stream */}
      <div className="discuss__stream">
        {turns.length === 0 && !streamingText && (
          <div className="discuss__empty">
            <div className="discuss__empty-icon">
              <IconChat size={32} />
            </div>
            <h3>与 AI 探讨你的故事</h3>
            <p>
              深化人物、推演情节、设计冲突、布局伏笔……
              <br />
              在正式动笔前，先与 AI 自由脑暴。
            </p>
            <div className="discuss__quick-topics">
              {QUICK_TOPICS.map((t) => (
                <button
                  key={t.label}
                  className="discuss__topic-chip"
                  onClick={() => handleQuickStart(t.prompt)}
                  disabled={!activeModel || sending}
                >
                  {t.label}
                </button>
              ))}
            </div>
            {activeModel && (
              <div className="discuss__flow-hint">
                <IconArrowRight size={13} />
                探讨完成后，可切换到「创作」标签开始正式写作
              </div>
            )}
          </div>
        )}

        {turns.map((t, i) => (
          <div key={i} className={`discuss__bubble discuss__bubble--${t.role}`}>
            <div className="discuss__bubble-who">
              {t.role === "user" ? (
                <>
                  <IconUser size={14} />
                  你
                </>
              ) : (
                <>
                  <IconCompass size={14} />
                  AI 策划顾问
                </>
              )}
            </div>
            <div className="discuss__bubble-text">{t.content}</div>
          </div>
        ))}

        {streamingText && (
          <div className="discuss__bubble discuss__bubble--assistant discuss__bubble--streaming">
            <div className="discuss__bubble-who">
              <IconCompass size={14} />
              AI 策划顾问
            </div>
            <div className="discuss__bubble-text">
              {streamingText}
              <span className="caret" />
            </div>
          </div>
        )}

        {sending && !streamingText && (
          <div className="discuss__bubble discuss__bubble--assistant">
            <div className="discuss__bubble-who">
              <IconCompass size={14} />
              AI 策划顾问
            </div>
            <div className="discuss__bubble-text">
              <Spinner size={14} />
            </div>
          </div>
        )}
      </div>

      {/* Input composer */}
      <div className="discuss__composer">
        <textarea
          className="discuss__input"
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              void sendMessage();
            }
          }}
          placeholder="说说你的想法，或问个问题…"
          rows={3}
          disabled={sending || !activeModel}
          spellCheck={false}
        />
        <div className="discuss__composer-actions">
          <div className="discuss__hint">
            <kbd>Enter</kbd> 发送 · <kbd>Shift + Enter</kbd> 换行
          </div>
          <button
            className="btn btn--primary discuss__send-btn"
            onClick={() => void sendMessage()}
            disabled={!draft.trim() || sending || !activeModel}
          >
            {sending ? <Spinner size={14} /> : <IconBrush size={16} />}
            {sending ? "发送中…" : "发送"}
          </button>
        </div>
      </div>
    </div>
  );
}
