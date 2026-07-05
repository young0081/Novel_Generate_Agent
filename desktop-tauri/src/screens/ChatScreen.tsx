// 探讨 — a dedicated conversation界面 with the AI creative partner. A standalone,
// streaming multi-turn chat (the reply types out live token by token) for
// brainstorming worldview / characters / plot / tone before and during writing.
// Tool-less, no agent loop; uses the reusable Conversation component.

import { useCallback, useEffect, useState } from "react";
import Panel from "../components/Panel";
import { IconStar, IconProviders } from "../components/icons";
import { describeError, isDesktop } from "../lib/core";
import { useToast } from "../components/Toast";
import { chatStream, type ChatMessage } from "../lib/studio";
import { isNoProviderError } from "../lib/agentRun";
import { getProviders } from "../lib/providers";
import { getSession } from "../lib/sessions";
import Conversation, {
  type ConversationTurn,
} from "../components/agent/Conversation";
import type { ScreenId } from "../lib/screens";

interface ChatScreenProps {
  onNavigate?: (id: ScreenId) => void;
  /** When set, load this saved discuss session and continue it. */
  resumeId?: string;
  /** Called once a resume request has been consumed. */
  onResumed?: () => void;
}

const SYSTEM =
  "你是一位资深的同人小说策划与写作伙伴。你的任务是与作者一起头脑风暴、打磨作品的" +
  "世界观、人物、情节、基调与文风。请用中文，像创作搭档一样提出有启发性的追问、给出" +
  "具体可落地的建议与多种可能方向，帮助作者把模糊的灵感逐步理清。回答精炼、紧扣作者" +
  "的话题，不要空泛。";

const GREETING =
  "我是你的创作搭档。无论是原作设定、人物动机、情节走向，还是某段文风的拿捏，都可以" +
  "丢给我，我们一来一回地把它聊透。";

const SUGGESTIONS = [
  "帮我理清这部作品的核心冲突",
  "主角的动机还能怎么深化？",
  "给我三个不同走向的开篇方案",
];

interface ActiveModel {
  provider: string;
  model: string;
}

export default function ChatScreen({
  onNavigate,
  resumeId,
  onResumed,
}: ChatScreenProps) {
  const toast = useToast();
  const [active, setActive] = useState<ActiveModel | null>(null);
  const [providerChecked, setProviderChecked] = useState(false);

  const [turns, setTurns] = useState<ConversationTurn[]>([]);
  const [draft, setDraft] = useState("");
  const [sending, setSending] = useState(false);
  const [streamingText, setStreamingText] = useState("");
  // the discuss session being appended to (persists the thread for resume)
  const [sessionId, setSessionId] = useState<string | null>(null);

  // Resume a saved discuss session: load its turns and keep appending to it.
  useEffect(() => {
    if (!resumeId) return;
    let alive = true;
    void (async () => {
      try {
        const rec = await getSession(resumeId);
        if (!alive) return;
        const loaded: ConversationTurn[] = (rec.session.messages ?? [])
          .filter((m) => m.role === "user" || m.role === "assistant")
          .map((m) => ({
            role: m.role as "user" | "assistant",
            content: m.content,
          }));
        setTurns(loaded);
        setSessionId(rec.session.id);
        setDraft("");
        setStreamingText("");
        toast.ok(`已载入会话「${rec.session.title || "未命名"}」`);
      } catch (e) {
        if (alive) toast.err(`载入会话失败：${describeError(e)}`);
      } finally {
        onResumed?.();
      }
    })();
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [resumeId]);

  const loadActive = useCallback(async () => {
    if (!isDesktop()) {
      setProviderChecked(true);
      return;
    }
    try {
      const s = await getProviders();
      const prov = s.providers.find((p) => p.id === s.active_provider);
      if (prov && s.active_model) {
        setActive({ provider: prov.name || "（未命名）", model: s.active_model });
      } else {
        setActive(null);
      }
    } catch {
      setActive(null);
    } finally {
      setProviderChecked(true);
    }
  }, []);

  useEffect(() => {
    void loadActive();
  }, [loadActive]);

  const send = useCallback(
    async (override?: string) => {
      const text = (override ?? draft).trim();
      if (!text || sending) return;
      const history: ConversationTurn[] = [
        ...turns,
        { role: "user", content: text },
      ];
      setTurns(history);
      setDraft("");
      setSending(true);
      setStreamingText("");

      const wire: ChatMessage[] = [{ role: "system", content: SYSTEM }];
      for (const t of history) wire.push({ role: t.role, content: t.content });

      let acc = "";
      try {
        const res = await chatStream(
          wire,
          (d) => {
            acc += d;
            setStreamingText(acc);
          },
          sessionId ?? undefined,
        );
        const clean = (res.text.trim() || acc.trim()) || "（模型返回了空内容）";
        setTurns((prev) => [...prev, { role: "assistant", content: clean }]);
        setSessionId(res.sessionId);
        if (!active) void loadActive();
      } catch (e) {
        const msg = describeError(e);
        setTurns((prev) => {
          const next = [...prev];
          if (next.length && next[next.length - 1].role === "user") next.pop();
          return next;
        });
        setDraft(text);
        if (isNoProviderError(msg)) {
          toast.err("尚未选用模型，请先到「供应商」配置");
        } else {
          toast.err(`探讨失败：${msg}`);
        }
      } finally {
        setStreamingText("");
        setSending(false);
      }
    },
    [draft, sending, turns, active, loadActive, toast, sessionId],
  );

  const clearChat = useCallback(() => {
    setTurns([]);
    setDraft("");
    setSessionId(null);
  }, []);

  const showProviderHint = providerChecked && !active && isDesktop();

  const toolbar = (
    <div className="toolbar plan-toolbar">
      <span className="toolbar__label">当前模型</span>
      {active ? (
        <span className="chip chip--accent">
          <IconStar size={11} />
          {active.provider} · {active.model}
        </span>
      ) : (
        <span className="chip">{providerChecked ? "尚未选用" : "检查中…"}</span>
      )}
      <button
        className="plan-toolbar__link"
        onClick={() => onNavigate?.("providers")}
        title="前往供应商配置"
      >
        <IconProviders size={13} />
        {active ? "切换 / 管理" : "去配置"}
      </button>
    </div>
  );

  const headerActions =
    turns.length > 0 ? (
      <button
        className="btn btn--ghost"
        onClick={clearChat}
        disabled={sending}
        title="清空对话"
      >
        清空对话
      </button>
    ) : undefined;

  return (
    <Panel
      title="探讨"
      en="Discuss"
      subtitle="与 AI 创作搭档一来一回 · 打磨世界观、人物、情节与文风"
      toolbar={toolbar}
      actions={headerActions}
    >
      <div className="scroll-area chat-screen">
        {showProviderHint && (
          <div className="callout callout--accent">
            <span className="callout__icon">
              <IconProviders size={20} />
            </span>
            <div className="callout__main">
              <h4 className="callout__title">还没有启用模型</h4>
              <p className="callout__text">
                配置一个供应商（填入 API Key 与模型）后，即可与 AI 实时探讨创作。
              </p>
            </div>
            <button
              className="btn btn--primary callout__action"
              onClick={() => onNavigate?.("providers")}
            >
              <IconProviders size={15} />
              去配置供应商
            </button>
          </div>
        )}

        <Conversation
          turns={turns}
          sending={sending}
          draft={draft}
          onDraft={setDraft}
          onSend={() => void send()}
          onSuggest={(s) => void send(s)}
          suggestions={SUGGESTIONS}
          greeting={GREETING}
          placeholder="说说你的想法，或问问该怎么处理某个设定…"
          assistantName="创作搭档"
          streamingText={streamingText}
        />
      </div>
    </Panel>
  );
}
