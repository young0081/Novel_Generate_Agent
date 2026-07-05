// A centered confirm dialog with the seal motif. Used for destructive
// actions like restoring a checkpoint.
//
// It mounts/unmounts around an internal "closing" window so both the open
// (spring up) and close (settle down) are animated; under prefers-reduced-
// motion the close window collapses to an instant via the stylesheet guard.

import { useEffect, useRef, useState, type ReactNode } from "react";
import Seal from "./Seal";
import { Spinner } from "./Spinner";

interface ConfirmModalProps {
  open: boolean;
  title: string;
  body: ReactNode;
  confirmLabel?: string;
  cancelLabel?: string;
  danger?: boolean;
  busy?: boolean;
  sealChar?: string;
  onConfirm: () => void;
  onCancel: () => void;
}

/** keep in step with the .modal-backdrop.is-closing animation duration */
const CLOSE_MS = 180;

export default function ConfirmModal({
  open,
  title,
  body,
  confirmLabel = "确认",
  cancelLabel = "取消",
  danger = false,
  busy = false,
  sealChar = "印",
  onConfirm,
  onCancel,
}: ConfirmModalProps) {
  // `mounted` keeps the node in the tree during the close animation;
  // `closing` toggles the exit classes.
  const [mounted, setMounted] = useState(open);
  const [closing, setClosing] = useState(false);
  const closeTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    if (open) {
      if (closeTimer.current) {
        clearTimeout(closeTimer.current);
        closeTimer.current = null;
      }
      setMounted(true);
      setClosing(false);
    } else if (mounted) {
      // begin the exit animation, then unmount
      setClosing(true);
      closeTimer.current = setTimeout(() => {
        setMounted(false);
        setClosing(false);
        closeTimer.current = null;
      }, CLOSE_MS);
    }
    return () => {
      if (closeTimer.current) {
        clearTimeout(closeTimer.current);
        closeTimer.current = null;
      }
    };
  }, [open, mounted]);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !busy) onCancel();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, busy, onCancel]);

  if (!mounted) return null;

  return (
    <div
      className={`modal-backdrop${closing ? " is-closing" : ""}`}
      onClick={() => {
        if (!busy) onCancel();
      }}
    >
      <div
        className={`modal${closing ? " is-closing" : ""}`}
        role="dialog"
        aria-modal="true"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="modal__seal">
          <Seal size={46} char={sealChar} tone="soft" />
        </div>
        <div className="modal__body">
          <h3 className="modal__title">{title}</h3>
          <div className="modal__text">{body}</div>
        </div>
        <div className="modal__foot">
          <button className="btn" onClick={onCancel} disabled={busy}>
            {cancelLabel}
          </button>
          <button
            className={`btn ${danger ? "btn--danger" : "btn--primary"}`}
            onClick={onConfirm}
            disabled={busy}
          >
            {busy ? <Spinner size={15} /> : null}
            {confirmLabel}
          </button>
        </div>
      </div>
    </div>
  );
}
