// Shared active-work context. Holds the current work + the library list, and
// exposes refresh/switch helpers so the title bar, library, and knowledge
// screens all stay in sync after a switch or create.

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import {
  listWorks,
  currentWork,
  openWork,
  type WorkMeta,
  type WorkSummary,
} from "../lib/works";
import { describeError, isDesktop } from "../lib/core";

interface WorkContextValue {
  /** The active work's full metadata, or null when none / loading. */
  current: WorkMeta | null;
  /** Every work in the library, newest-updated first. */
  works: WorkSummary[];
  loading: boolean;
  error: string | null;
  /** Re-pull the library + active work from the backend. */
  refresh: () => Promise<void>;
  /** Switch the active work (rebuilds the engine backend-side). */
  switchTo: (id: string) => Promise<void>;
}

const WorkContext = createContext<WorkContextValue | null>(null);

export function WorkProvider({ children }: { children: ReactNode }) {
  const [current, setCurrent] = useState<WorkMeta | null>(null);
  const [works, setWorks] = useState<WorkSummary[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const stateRequestSeq = useRef(0);
  const switchChainRef = useRef<Promise<void>>(Promise.resolve());

  const refresh = useCallback(async () => {
    const request = ++stateRequestSeq.current;
    if (!isDesktop()) {
      if (request === stateRequestSeq.current) setLoading(false);
      return;
    }
    try {
      if (request === stateRequestSeq.current) setError(null);
      const [list, cur] = await Promise.all([listWorks(), currentWork()]);
      if (request !== stateRequestSeq.current) return;
      setWorks(list);
      setCurrent(cur);
    } catch (refreshError) {
      if (request === stateRequestSeq.current) {
        setError(describeError(refreshError));
      }
    } finally {
      if (request === stateRequestSeq.current) setLoading(false);
    }
  }, []);

  const switchTo = useCallback(
    (id: string): Promise<void> => {
      const request = ++stateRequestSeq.current;
      const transition = switchChainRef.current.then(async () => {
        const list = await openWork(id);
        const cur = await currentWork();
        if (request !== stateRequestSeq.current) return;
        setWorks(list);
        setCurrent(cur);
        setError(null);
      });
      switchChainRef.current = transition.catch(() => undefined);
      return transition;
    },
    [],
  );

  useEffect(() => {
    void refresh();
  }, [refresh]);

  return (
    <WorkContext.Provider value={{ current, works, loading, error, refresh, switchTo }}>
      {children}
    </WorkContext.Provider>
  );
}

export function useWork(): WorkContextValue {
  const ctx = useContext(WorkContext);
  if (!ctx) throw new Error("useWork must be used within a WorkProvider");
  return ctx;
}
