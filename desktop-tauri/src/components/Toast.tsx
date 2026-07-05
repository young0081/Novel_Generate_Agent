// Lightweight toast system: a provider + a `useToast()` hook.

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { IconCheck, IconWarn, IconInfo } from "./icons";

type ToastKind = "ok" | "err" | "info";

interface ToastItem {
  id: number;
  kind: ToastKind;
  message: string;
  /** true once the toast is animating out, before it is unmounted */
  leaving?: boolean;
}

/** how long the exit animation runs before the node is removed (ms). Kept in
 *  step with the .toast.is-leaving animation duration in the stylesheet. */
const EXIT_MS = 240;

interface ToastApi {
  push: (message: string, kind?: ToastKind) => void;
  ok: (message: string) => void;
  err: (message: string) => void;
  info: (message: string) => void;
}

const ToastContext = createContext<ToastApi | null>(null);

export function ToastProvider({ children }: { children: ReactNode }) {
  const [items, setItems] = useState<ToastItem[]>([]);
  const seq = useRef(0);
  const timers = useRef<Record<number, ReturnType<typeof setTimeout>>>({});
  const exitTimers = useRef<Record<number, ReturnType<typeof setTimeout>>>({});

  // Two-phase removal: mark the toast leaving (plays the exit animation),
  // then drop it from the list once the animation window has elapsed.
  const remove = useCallback((id: number) => {
    const t = timers.current[id];
    if (t) {
      clearTimeout(t);
      delete timers.current[id];
    }
    if (exitTimers.current[id]) return; // already leaving
    setItems((prev) =>
      prev.map((it) => (it.id === id ? { ...it, leaving: true } : it)),
    );
    exitTimers.current[id] = setTimeout(() => {
      setItems((prev) => prev.filter((it) => it.id !== id));
      delete exitTimers.current[id];
    }, EXIT_MS);
  }, []);

  const push = useCallback(
    (message: string, kind: ToastKind = "info") => {
      const id = ++seq.current;
      setItems((prev) => [...prev, { id, kind, message }]);
      timers.current[id] = setTimeout(() => remove(id), 3200);
    },
    [remove],
  );

  useEffect(() => {
    const map = timers.current;
    const exits = exitTimers.current;
    return () => {
      Object.values(map).forEach(clearTimeout);
      Object.values(exits).forEach(clearTimeout);
    };
  }, []);

  const api: ToastApi = {
    push,
    ok: (m) => push(m, "ok"),
    err: (m) => push(m, "err"),
    info: (m) => push(m, "info"),
  };

  return (
    <ToastContext.Provider value={api}>
      {children}
      <div className="toast-wrap" role="status" aria-live="polite">
        {items.map((t) => (
          <div
            key={t.id}
            className={`toast toast--${t.kind}${t.leaving ? " is-leaving" : ""}`}
            onClick={() => remove(t.id)}
            role="button"
            title="点击关闭"
          >
            {t.kind === "ok" ? (
              <IconCheck size={16} />
            ) : t.kind === "err" ? (
              <IconWarn size={16} />
            ) : (
              <IconInfo size={16} />
            )}
            <span>{t.message}</span>
          </div>
        ))}
      </div>
    </ToastContext.Provider>
  );
}

export function useToast(): ToastApi {
  const ctx = useContext(ToastContext);
  if (!ctx) {
    throw new Error("useToast 必须在 ToastProvider 内使用");
  }
  return ctx;
}
