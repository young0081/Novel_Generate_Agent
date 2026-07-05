// 对话界面 — a reusable, self-contained chat thread + composer for talking with
// the model (used by the 策划 screen's 与 AI 探讨). Renders a calm greeting with
// quick-start suggestions before the first turn, directional bubbles, a typing
// indicator while the model replies, and an Enter-to-send composer.

import {
  useEffect,
  useRef,
  type KeyboardEvent,
  type ReactNode,
} from "react";
import { Spinner } from "../Spinner";
import { IconUser, IconCompass, IconChat, IconInfo, BrushStroke } from "../icons";

export interface ConversationTurn {
  role: "user" | "assistant";
  content: string;
}

interface ConversationProps {
  turns: ConversationTurn[];
  sending: boolean;
  draft: string;
  onDraft: (value: string) => void;
  onSend: () => void;
  /** Click a suggestion chip to open the thread with it. */
  onSuggest?: (text: string) => void;
  suggestions?: string[];
  greeting?: ReactNode;
  placeholder?: string;
  /** Disable the composer entirely (e.g. while a generation run is in flight). */
  disabled?: boolean;
  /** Label for the assistant in bubbles + typing row. */
  assistantName?: string;
  /**
   * The reply currently streaming in (token by token). While `sending`, if this
   * is non-empty it renders as a live assistant bubble with an ink caret;
   * otherwise the typing dots show (we're still waiting for the first token).
   */
  streamingText?: string;
}

export default function Conversation({
  turns,
  sending,
  draft,
  onDraft,
  onSend,
  onSuggest,
  suggestions = [],
  greeting,
  placeholder = "说说你的想法…",
  disabled = false,
  assistantName = "策划伙伴",
  streamingText = "",
}: ConversationProps) {
  const tailRef = useRef<HTMLDivElement>(null);

  // keep the newest turn / typing indicator / streaming reply in view
  useEffect(() => {
    tailRef.current?.scrollIntoView({ behavior: "smooth", block: "end" });
  }, [turns, sending, streamingText]);

  const onKey = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      if (!sending && !disabled) onSend();
    }
  };

  const empty = turns.length === 0 && !sending;

  return (
    <div className="conv">
      <div className="conv__thread">
        {empty ? (
          <div className="conv__greeting">
            <BrushStroke className="conv__greeting-flourish" aria-hidden="true" />
            <span className="conv__greeting-mark">
              <IconCompass size={22} />
            </span>
            <div className="conv__greeting-text">{greeting}</div>
            {suggestions.length > 0 && onSuggest && (
              <div className="conv__suggests">
                {suggestions.map((s) => (
                  <button
                    key={s}
                    type="button"
                    className="conv-suggest"
                    onClick={() => onSuggest(s)}
                    disabled={sending || disabled}
                    title="以此开启对话"
                  >
                    {s}
                  </button>
                ))}
              </div>
            )}
          </div>
        ) : (
          turns.map((t, i) => (
            <div key={i} className={`conv-bubble conv-bubble--${t.role}`}>
              <span className="conv-bubble__who">
                {t.role === "user" ? (
                  <IconUser size={13} />
                ) : (
                  <IconCompass size={13} />
                )}
                {t.role === "user" ? "你" : assistantName}
              </span>
              <div className="conv-bubble__text">{t.content}</div>
            </div>
          ))
        )}
        {sending &&
          (streamingText ? (
            <div className="conv-bubble conv-bubble--assistant">
              <span className="conv-bubble__who">
                <IconCompass size={13} />
                {assistantName}
              </span>
              <div className="conv-bubble__text">
                {streamingText}
                <span className="ink-caret" aria-hidden="true" />
              </div>
            </div>
          ) : (
            <div className="conv-bubble conv-bubble--assistant conv-bubble--typing">
              <span className="conv-bubble__who">
                <IconCompass size={13} />
                {assistantName}
              </span>
              <div className="conv-typing" aria-label="对方正在输入">
                <span className="conv-typing__dot" />
                <span className="conv-typing__dot" />
                <span className="conv-typing__dot" />
              </div>
            </div>
          ))}
        <div ref={tailRef} />
      </div>

      <div className="conv__composer">
        <textarea
          className="textarea conv__input"
          value={draft}
          onChange={(e) => onDraft(e.target.value)}
          onKeyDown={onKey}
          placeholder={placeholder}
          spellCheck={false}
          disabled={sending || disabled}
        />
        <button
          className="btn btn--primary conv__send"
          onClick={onSend}
          disabled={sending || disabled || !draft.trim()}
          title="发送"
        >
          {sending ? <Spinner size={16} /> : <IconChat size={16} />}
          发送
        </button>
      </div>
      <p className="field__hint conv__hint">
        <IconInfo size={12} className="field__hint-icon" />
        按 <span className="kbd">Enter</span> 发送，
        <span className="kbd">Shift + Enter</span> 换行。探讨只是构思，不会写入记忆库。
      </p>
    </div>
  );
}
