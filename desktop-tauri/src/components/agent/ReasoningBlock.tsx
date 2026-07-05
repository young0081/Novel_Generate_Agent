// 模型推理 — renders one model turn's reasoning text as a tagged block with a
// live ink-brush caret while it streams. Long thoughts collapse behind a
// "展开全文" toggle so a verbose chain-of-thought never floods the feed.

import { memo, useMemo, useState } from "react";
import { IconThought } from "../icons";

interface ReasoningBlockProps {
  text: string;
  /** This is the in-flight step — show the blinking caret. */
  active?: boolean;
}

const COLLAPSE_AT = 480;

function ReasoningBlock({ text, active }: ReasoningBlockProps) {
  const [open, setOpen] = useState(false);
  const trimmed = useMemo(() => text.trim(), [text]);
  const long = trimmed.length > COLLAPSE_AT;
  const shown = useMemo(
    () => (long && !open ? `${trimmed.slice(0, COLLAPSE_AT)}…` : trimmed),
    [long, open, trimmed],
  );

  return (
    <div className="reasoning">
      <span className="reasoning__tag">
        <IconThought size={12} />
        {active ? "推理中" : "推理"}
      </span>
      {trimmed ? (
        <div className="reasoning__text">
          {shown}
          {active && <span className="ink-caret" aria-hidden="true" />}
          {long && (
            <button
              type="button"
              className="reasoning__more"
              onClick={() => setOpen((v) => !v)}
            >
              {open ? "收起" : "展开全文"}
            </button>
          )}
        </div>
      ) : (
        <div className="reasoning__text reasoning__text--muted">
          （正在斟酌…）
          {active && <span className="ink-caret" aria-hidden="true" />}
        </div>
      )}
    </div>
  );
}

export default memo(ReasoningBlock);
