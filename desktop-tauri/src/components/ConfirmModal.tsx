// A centered confirm dialog with the seal motif. Used for destructive
// actions like restoring a checkpoint.
//
// It mounts/unmounts around an internal "closing" window so both the open
// (spring up) and close (settle down) are animated; under prefers-reduced-
// motion the close window collapses to an instant via the stylesheet guard.

import { useEffect, useId, useRef, useState, type ReactNode } from "react";
import Seal from "./Seal";
import { Spinner } from "./Spinner";
import { layerExitDelay } from "../lib/dialogLayer";

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
  const cancelRef = useRef<HTMLButtonElement>(null);
  const modalRef = useRef<HTMLDivElement>(null);
  const titleId = useId();
  const bodyId = useId();

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
      }, layerExitDelay(CLOSE_MS));
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
    const previouslyFocused = document.activeElement as HTMLElement | null;
    const frame = window.requestAnimationFrame(() => {
      const initialTarget = cancelRef.current?.disabled ? modalRef.current : cancelRef.current;
      initialTarget?.focus();
    });
    return () => {
      window.cancelAnimationFrame(frame);
      if (previouslyFocused?.isConnected) previouslyFocused.focus();
    };
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        if (!busy) onCancel();
        return;
      }
      if (e.key !== "Tab") return;

      const dialog = modalRef.current;
      if (!dialog) return;
      const focusable = Array.from(
        dialog.querySelectorAll<HTMLElement>(
          'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])',
        ),
      );
      if (focusable.length === 0) {
        e.preventDefault();
        dialog.focus();
        return;
      }

      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      const active = document.activeElement;
      if (
        e.shiftKey &&
        (active === first || active === dialog || !dialog.contains(active))
      ) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && active === last) {
        e.preventDefault();
        first.focus();
      }
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
        ref={modalRef}
        className={`modal${closing ? " is-closing" : ""}`}
        role="alertdialog"
        aria-modal="true"
        aria-labelledby={titleId}
        aria-describedby={bodyId}
        aria-busy={busy || undefined}
        tabIndex={-1}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="modal__seal" aria-hidden="true">
          <Seal size={46} char={sealChar} tone="soft" />
        </div>
        <div className="modal__body">
          <h3 className="modal__title" id={titleId}>
            {title}
          </h3>
          <div className="modal__text" id={bodyId}>
            {body}
          </div>
        </div>
        <div className="modal__foot">
          <button
            type="button"
            ref={cancelRef}
            className="btn"
            onClick={onCancel}
            disabled={busy}
          >
            {cancelLabel}
          </button>
          <button
            type="button"
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
