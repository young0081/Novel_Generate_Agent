"use client";

import {
  createContext,
  useCallback,
  useContext,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";

type ToastKind = "success" | "error" | "info";

interface ToastItem {
  id: number;
  kind: ToastKind;
  message: string;
  /** Drives the enter/exit animation: false = entering/visible, true = leaving. */
  leaving: boolean;
}

interface ToastApi {
  success: (message: string) => void;
  error: (message: string) => void;
  info: (message: string) => void;
}

const ToastContext = createContext<ToastApi | null>(null);

const AUTO_DISMISS_MS = 4200;
const EXIT_ANIM_MS = 220;

const ICON: Record<ToastKind, string> = {
  success: "✓",
  error: "!",
  info: "i",
};

/**
 * Hand-rolled toast system. Toasts stack in the top-right corner, animate in,
 * auto-dismiss after a few seconds (animating out), and can be dismissed early
 * by clicking them.
 */
export function ToastProvider({ children }: { children: ReactNode }) {
  const [toasts, setToasts] = useState<ToastItem[]>([]);
  const nextId = useRef(1);
  // Track timers so we never leak / double-fire after manual dismiss.
  const timers = useRef<Map<number, ReturnType<typeof setTimeout>>>(new Map());

  const remove = useCallback((id: number) => {
    setToasts((cur) => cur.filter((t) => t.id !== id));
    const t = timers.current.get(id);
    if (t) {
      clearTimeout(t);
      timers.current.delete(id);
    }
  }, []);

  const dismiss = useCallback(
    (id: number) => {
      // Start the exit animation, then unmount after it finishes.
      setToasts((cur) => cur.map((t) => (t.id === id ? { ...t, leaving: true } : t)));
      const existing = timers.current.get(id);
      if (existing) clearTimeout(existing);
      const t = setTimeout(() => remove(id), EXIT_ANIM_MS);
      timers.current.set(id, t);
    },
    [remove],
  );

  const push = useCallback(
    (kind: ToastKind, message: string) => {
      const id = nextId.current++;
      setToasts((cur) => [...cur, { id, kind, message, leaving: false }]);
      const t = setTimeout(() => dismiss(id), AUTO_DISMISS_MS);
      timers.current.set(id, t);
    },
    [dismiss],
  );

  const api = useMemo<ToastApi>(
    () => ({
      success: (m) => push("success", m),
      error: (m) => push("error", m),
      info: (m) => push("info", m),
    }),
    [push],
  );

  return (
    <ToastContext.Provider value={api}>
      {children}
      <div className="toast-stack" aria-live="polite" aria-atomic="false">
        {toasts.map((t) => (
          <button
            key={t.id}
            type="button"
            className={`toast toast-${t.kind}${t.leaving ? " leaving" : ""}`}
            onClick={() => dismiss(t.id)}
            title="点击关闭"
          >
            <span className="toast-icon" aria-hidden>
              {ICON[t.kind]}
            </span>
            <span className="toast-msg">{t.message}</span>
          </button>
        ))}
      </div>
    </ToastContext.Provider>
  );
}

export function useToast(): ToastApi {
  const ctx = useContext(ToastContext);
  if (!ctx) {
    throw new Error("useToast 必须在 <ToastProvider> 内部使用");
  }
  return ctx;
}
