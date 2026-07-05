// IdeAiPanel — AI assistant panel embedded in the IDE.
// Context-aware: sends current file content + selection + user message to AI.

import { useCallback, useEffect, useRef, useState } from "react";
import { chatStream, type ChatMessage } from "../../lib/studio";
import { IconBrush, IconUser, IconProviders, IconRefresh } from "../icons";
import { Spinner } from "../Spinner";
import { useToast } from "../Toast";
import { getProviders } from "../../lib/providers";

interface IdeAiPanelProps {
  filePath: string | null;
  getFileContent: () => string;  // callback to read current editor content
}

interface Turn {
  role: "user" | "assistant";
  content: string;
  streaming?: boolean;
}

export default function IdeAiPanel({ filePath, getFileContent }: IdeAiPanelProps) {
  const toast = useToast();
  const [turns, setTurns] = useState<Turn[]>([]);
  const [draft, setDraft] = useState("");
  const [sending, setSending] = useState(false);
  const [sessionId, setSessionId] = useState<string | null>(null);
  const [hasProvider, setHasProvider] = useState<boolean | null>(null);
  const bottomRef = useRef<HTMLDivElement>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    getProviders()
      .then((p) => setHasProvider(p.providers.length > 0))
      .catch(() => setHasProvider(false));
  }, []);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [turns]);

  const send = useCallback(async () => {
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
    setTurns((prev) => [...prev, { role: "assistant", content: "", streaming: true }]);

    try {
      const result = await chatStream(
        history,
        (chunk: string) => {
          accumulated += chunk;
          setTurns((prev) => {
            const next = [...prev];
            const last = next[next.length - 1];
            if (last?.role === "assistant") {
              next[next.length - 1] = { ...last, content: accumulated, streaming: true };
            }
            return next;
          });
        },
        sessionId ?? undefined,
      );
      setSessionId(result.sessionId);
      setTurns((prev) => {
        const next = [...prev];
        const last = next[next.length - 1];
        if (last?.role === "assistant") {
          next[next.length - 1] = { ...last, content: result.text || accumulated, streaming: false };
        }
        return next;
      });
    } catch (e) {
      setTurns((prev) => prev.filter((t) => !t.streaming));
      toast.err("AI 回复失败，请重试");
    } finally {
      setSending(false);
    }
  }, [draft, sending, turns, filePath, getFileContent, sessionId, toast]);

  const handleKey = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        send();
      }
    },
    [send],
  );

  const clearChat = useCallback(() => {
    setTurns([]);
    setSessionId(null);
  }, []);

  if (hasProvider === false) {
    return (
      <div className="ide-ai__no-provider">
        <IconProviders size={24} />
        <p>请先配置 AI 供应商</p>
      </div>
    );
  }

  return (
    <div className="ide-ai">
      <div className="ide-ai__header">
        <IconBrush size={14} />
        <span>AI 助手</span>
        {turns.length > 0 && (
          <button className="ide-ai__clear" onClick={clearChat} title="清空对话">
            <IconRefresh size={12} />
          </button>
        )}
      </div>

      <div className="ide-ai__messages">
        {turns.length === 0 && (
          <div className="ide-ai__welcome">
            <p>你可以：</p>
            <ul>
              <li>「续写这一段」</li>
              <li>「把这里改得更有张力」</li>
              <li>「给我建议一个转折点」</li>
              <li>「检查这段有没有前后矛盾」</li>
            </ul>
            {filePath && <p className="ide-ai__file-hint">当前文件：{filePath}</p>}
          </div>
        )}
        {turns.map((turn, i) => (
          <div key={i} className={`ide-ai__turn ide-ai__turn--${turn.role}`}>
            <span className="ide-ai__avatar">
              {turn.role === "user" ? <IconUser size={13} /> : <IconBrush size={13} />}
            </span>
            <div className="ide-ai__bubble">
              {turn.content || (turn.streaming ? <span className="ide-ai__caret">▍</span> : "")}
            </div>
          </div>
        ))}
        <div ref={bottomRef} />
      </div>

      <div className="ide-ai__input-row">
        <textarea
          ref={textareaRef}
          className="ide-ai__input"
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={handleKey}
          placeholder="问 AI… (Enter 发送, Shift+Enter 换行)"
          rows={3}
          disabled={sending}
        />
        <button
          className="ide-ai__send"
          onClick={send}
          disabled={!draft.trim() || sending}
        >
          {sending ? <Spinner size={14} /> : <IconBrush size={14} />}
        </button>
      </div>
    </div>
  );
}
