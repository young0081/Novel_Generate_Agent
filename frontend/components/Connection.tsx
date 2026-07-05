"use client";

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";

import { rpc } from "@/lib/rpcClient";

export type ConnState = "checking" | "ok" | "bad";

interface ConnApi {
  state: ConnState;
  /** Re-ping the core; flips `state` to checking → ok/bad. Returns true if up. */
  ping: () => Promise<boolean>;
  /**
   * Panels call this when an action fails. If the message looks like the core is
   * unreachable, the shared connection banner is shown (state -> bad).
   */
  reportError: (message: string) => void;
}

const ConnContext = createContext<ConnApi | null>(null);

/** Heuristic: does this error indicate the Rust core couldn't be reached at all? */
function looksUnreachable(message: string): boolean {
  const m = message.toLowerCase();
  const coreDown = m.includes("core") && (m.includes("unreachable") || m.includes("not running"));
  return (
    m.includes("failed to fetch") ||
    m.includes("networkerror") ||
    m.includes("load failed") ||
    m.includes("[-32000]") || // proxy: core bridge / spawn failure
    m.includes("econnrefused") ||
    m.includes("enoent") ||
    m.includes("502") ||
    coreDown
  );
}

export function ConnectionProvider({ children }: { children: ReactNode }) {
  const [state, setState] = useState<ConnState>("checking");
  const alive = useRef(true);

  const ping = useCallback(async () => {
    setState("checking");
    try {
      const r = await rpc<string>("ping");
      const ok = r === "pong";
      if (alive.current) setState(ok ? "ok" : "bad");
      return ok;
    } catch {
      if (alive.current) setState("bad");
      return false;
    }
  }, []);

  const reportError = useCallback((message: string) => {
    if (looksUnreachable(message)) {
      setState("bad");
    }
  }, []);

  useEffect(() => {
    alive.current = true;
    void ping();
    return () => {
      alive.current = false;
    };
  }, [ping]);

  const api = useMemo<ConnApi>(() => ({ state, ping, reportError }), [state, ping, reportError]);

  return <ConnContext.Provider value={api}>{children}</ConnContext.Provider>;
}

export function useConnection(): ConnApi {
  const ctx = useContext(ConnContext);
  if (!ctx) {
    throw new Error("useConnection 必须在 <ConnectionProvider> 内部使用");
  }
  return ctx;
}
