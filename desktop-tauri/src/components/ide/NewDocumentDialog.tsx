import {
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
} from "react";
import { IconFile, IconFolder } from "../icons";

export type NewDocumentKind = "chapter" | "note";

export interface NewDocumentDraft {
  kind: NewDocumentKind;
  path: string;
  title: string;
}

interface NewDocumentDialogProps {
  open: boolean;
  busy?: boolean;
  onCancel: () => void;
  onCreate: (draft: NewDocumentDraft) => void;
}

const INVALID_NAME = /[<>:"/\\|?*\u0000-\u001f]/;

function normalizeTitle(value: string): string {
  return value.trim().replace(/\.(?:md|markdown|txt)$/i, "");
}

export default function NewDocumentDialog({
  open,
  busy = false,
  onCancel,
  onCreate,
}: NewDocumentDialogProps) {
  const [kind, setKind] = useState<NewDocumentKind>("chapter");
  const [name, setName] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);
  const dialogRef = useRef<HTMLDivElement>(null);
  const chapterRef = useRef<HTMLButtonElement>(null);
  const noteRef = useRef<HTMLButtonElement>(null);
  const busyRef = useRef(busy);
  const cancelCallbackRef = useRef(onCancel);
  busyRef.current = busy;
  cancelCallbackRef.current = onCancel;
  const titleId = useId();
  const helpId = useId();
  const title = normalizeTitle(name);
  const directory = kind === "chapter" ? "book" : "notes";
  const path = `${directory}/${title || (kind === "chapter" ? "未命名章节" : "未命名资料")}.md`;
  const validationError = useMemo(() => {
    if (!name.trim()) return null;
    if (!title || title === "." || title === "..") return "请输入有效名称";
    if (INVALID_NAME.test(title)) return "名称不能包含 < > : \" / \\ | ? *";
    return null;
  }, [name, title]);

  useEffect(() => {
    if (!open) return;
    setKind("chapter");
    setName("");
    const previouslyFocused = document.activeElement as HTMLElement | null;
    const frame = window.requestAnimationFrame(() => inputRef.current?.focus());

    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        if (!busyRef.current) cancelCallbackRef.current();
        return;
      }
      if (event.key !== "Tab") return;
      const dialog = dialogRef.current;
      if (!dialog) return;
      const focusable = Array.from(
        dialog.querySelectorAll<HTMLElement>(
          'button:not([disabled]), input:not([disabled]), [tabindex]:not([tabindex="-1"])',
        ),
      );
      if (focusable.length === 0) return;
      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    };

    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.cancelAnimationFrame(frame);
      window.removeEventListener("keydown", onKeyDown);
      if (previouslyFocused?.isConnected) previouslyFocused.focus();
    };
  }, [open]);

  if (!open) return null;

  const submit = () => {
    if (!title || validationError || busy) return;
    onCreate({ kind, path: `${directory}/${title}.md`, title });
  };

  const onKindKeyDown = (event: ReactKeyboardEvent<HTMLButtonElement>) => {
    let next: NewDocumentKind | null = null;
    if (event.key === "ArrowLeft" || event.key === "ArrowUp" || event.key === "Home") {
      next = "chapter";
    } else if (event.key === "ArrowRight" || event.key === "ArrowDown" || event.key === "End") {
      next = "note";
    }
    if (!next) return;
    event.preventDefault();
    setKind(next);
    (next === "chapter" ? chapterRef.current : noteRef.current)?.focus();
  };

  return (
    <div
      className="modal-backdrop"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget && !busy) onCancel();
      }}
    >
      <div
        ref={dialogRef}
        className="modal modal--new-document"
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        aria-describedby={helpId}
        aria-busy={busy || undefined}
      >
        <form
          className="new-document"
          onSubmit={(event) => {
            event.preventDefault();
            submit();
          }}
        >
          <header className="new-document__header">
            <h2 id={titleId}>新建文稿</h2>
            <p id={helpId}>选择文稿类型并命名</p>
          </header>

          <div className="new-document__kind" role="radiogroup" aria-label="文稿类型">
            <button
              ref={chapterRef}
              type="button"
              role="radio"
              aria-checked={kind === "chapter"}
              tabIndex={kind === "chapter" ? 0 : -1}
              className={kind === "chapter" ? "is-active" : ""}
              onClick={() => setKind("chapter")}
              onKeyDown={onKindKeyDown}
              disabled={busy}
            >
              <IconFile size={15} />
              章节
            </button>
            <button
              ref={noteRef}
              type="button"
              role="radio"
              aria-checked={kind === "note"}
              tabIndex={kind === "note" ? 0 : -1}
              className={kind === "note" ? "is-active" : ""}
              onClick={() => setKind("note")}
              onKeyDown={onKindKeyDown}
              disabled={busy}
            >
              <IconFolder size={15} />
              资料
            </button>
          </div>

          <label className="new-document__field">
            <span>{kind === "chapter" ? "章节名" : "资料名"}</span>
            <input
              ref={inputRef}
              value={name}
              onChange={(event) => setName(event.target.value)}
              placeholder={kind === "chapter" ? "例如：第十二章 雨夜来客" : "例如：人物关系"}
              aria-invalid={!!validationError}
              disabled={busy}
            />
          </label>

          <div className="new-document__path" aria-live="polite">
            <span>保存至</span>
            <code>{path}</code>
          </div>
          {validationError && <p className="new-document__error">{validationError}</p>}

          <footer className="modal__foot">
            <button type="button" className="btn" onClick={onCancel} disabled={busy}>
              取消
            </button>
            <button
              type="submit"
              className="btn btn--primary"
              disabled={!title || !!validationError || busy}
            >
              {busy ? "正在创建..." : "创建并打开"}
            </button>
          </footer>
        </form>
      </div>
    </div>
  );
}
