"use client";

import { type CSSProperties, useCallback, useEffect, useRef, useState } from "react";

import Button, { Spinner } from "@/components/Button";
import { useConnection } from "@/components/Connection";
import EmptyState from "@/components/EmptyState";
import { listTools } from "@/lib/rpcClient";
import type { ToolSpec } from "@/lib/types";

export default function ToolCatalog() {
  const { reportError } = useConnection();
  const [tools, setTools] = useState<ToolSpec[]>([]);
  const [err, setErr] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const alive = useRef(true);

  const load = useCallback(async () => {
    setLoading(true);
    setErr(null);
    try {
      const t = await listTools();
      if (alive.current) setTools(t);
    } catch (e: unknown) {
      const message = e instanceof Error ? e.message : String(e);
      reportError(message);
      if (alive.current) setErr(message);
    } finally {
      if (alive.current) setLoading(false);
    }
  }, [reportError]);

  useEffect(() => {
    alive.current = true;
    void load();
    return () => {
      alive.current = false;
    };
  }, [load]);

  return (
    <div>
      <div className="card">
        <div className="card-head">
          <h3>
            已注册工具 <span className="badge">{tools.length}</span>
          </h3>
          <Button variant="ghost" onClick={load} loading={loading}>
            刷新
          </Button>
        </div>

        {loading && (
          <>
            <p className="hint loading-line">
              <Spinner /> 加载中…（首次会触发 Rust 核心编译/启动，请稍候十几秒）
            </p>
            <div className="tool-grid" aria-hidden>
              {Array.from({ length: 6 }).map((_, i) => (
                <div className="tool" key={i} style={{ "--i": i } as CSSProperties}>
                  <div className="skeleton-block" style={{ marginTop: 0 }}>
                    <div className="skeleton-line" style={{ width: "55%" }} />
                    <div className="skeleton-line" style={{ width: "90%" }} />
                    <div className="skeleton-line" style={{ width: "70%" }} />
                  </div>
                </div>
              ))}
            </div>
          </>
        )}

        {!loading && err && (
          <EmptyState
            icon="🔌"
            title="连接核心失败"
            hint={err}
          />
        )}

        {!loading && !err && tools.length === 0 && (
          <EmptyState icon="🧰" title="核心没有注册任何工具" />
        )}

        {!loading && !err && (
          <div className="tool-grid">
            {tools.map((t, i) => (
              <div
                className="tool"
                key={t.name}
                style={{ "--i": Math.min(i, 12) } as CSSProperties}
              >
                <div className="name">
                  {t.name}
                  {t.mutating && <span className="badge mut">写</span>}
                </div>
                <div className="desc">{t.description}</div>
                {t.capabilities.length > 0 && (
                  <div className="tool-caps">
                    {t.capabilities.map((c) => (
                      <span className="cap-chip" key={c}>
                        {c}
                      </span>
                    ))}
                  </div>
                )}
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
