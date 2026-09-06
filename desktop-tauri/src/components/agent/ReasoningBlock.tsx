// 模型推理 — renders one model turn's reasoning text as a tagged block with a
// live ink-brush caret while it streams. Long thoughts collapse behind a
// "展开全文" toggle so a verbose chain-of-thought never floods the feed.

import { memo, useMemo, useState } from "react";
import { IconPencil, IconThought } from "../icons";

interface ReasoningBlockProps {
  text: string;
  /** This is the in-flight step — show the blinking caret. */
  active?: boolean;
  /** Distinguishes internal deliberation from text arriving token by token. */
  mode?: "thinking" | "writing";
}

const COLLAPSE_AT = 320;

function ReasoningBlock({ text, active, mode = "thinking" }: ReasoningBlockProps) {
  const [open, setOpen] = useState(false);
  const trimmed = useMemo(() => text.trim(), [text]);
  const long = trimmed.length > COLLAPSE_AT;
  const shown = useMemo(
    () => (long && !active && !open ? `${trimmed.slice(0, COLLAPSE_AT)}…` : trimmed),
    [active, long, open, trimmed],
  );

  return (
    <div
      className={`reasoning${active ? " is-active" : ""}${mode === "writing" ? " is-writing" : ""}`}
      aria-busy={active}
    >
      <span className={`reasoning__tag${active ? " is-active" : ""}`}>
        {mode === "writing" ? <IconPencil size={12} /> : <IconThought size={12} />}
        {active ? (mode === "writing" ? "正在成文" : "正在构思") : "工作摘要"}
      </span>
      {trimmed ? (
        <div className="reasoning__text">
          {shown}
          {active && <span className="ink-caret" aria-hidden="true" />}
          {long && !active && (
            <button
              type="button"
              className="reasoning__more"
              onClick={() => setOpen((v) => !v)}
              aria-expanded={open}
            >
              {open ? "收起" : "展开全文"}
            </button>
          )}
        </div>
      ) : (
        <div className="reasoning__text reasoning__text--muted">
          正在梳理情节与人物动机…
          {active && <span className="ink-caret" aria-hidden="true" />}
        </div>
      )}
    </div>
  );
}

export default memo(ReasoningBlock);
