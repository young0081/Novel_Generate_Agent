// Shared active-work context. Holds the current work + the library list, and
// exposes refresh/switch helpers so the title bar, library, and knowledge
// screens all stay in sync after a switch or create.

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
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
import { isDesktop } from "../lib/core";

interface WorkContextValue {
  /** The active work's full metadata, or null when none / loading. */
  current: WorkMeta | null;
  /** Every work in the library, newest-updated first. */
  works: WorkSummary[];
  loading: boolean;
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

  const refresh = useCallback(async () => {
    if (!isDesktop()) {
      setLoading(false);
      return;
    }
    try {
      const [list, cur] = await Promise.all([listWorks(), currentWork()]);
      setWorks(list);
      setCurrent(cur);
    } catch {
      /* leave prior state */
    } finally {
      setLoading(false);
    }
  }, []);

  const switchTo = useCallback(
    async (id: string) => {
      const list = await openWork(id);
      setWorks(list);
      const cur = await currentWork();
      setCurrent(cur);
    },
    [],
  );

  useEffect(() => {
    void refresh();
  }, [refresh]);

  return (
    <WorkContext.Provider value={{ current, works, loading, refresh, switchTo }}>
      {children}
    </WorkContext.Provider>
  );
}

export function useWork(): WorkContextValue {
  const ctx = useContext(WorkContext);
  if (!ctx) throw new Error("useWork must be used within a WorkProvider");
  return ctx;
}
