"use client";

import { useState } from "react";

import Button from "@/components/Button";
import { useConnection } from "@/components/Connection";

export default function RpcConsole() {
  const { reportError } = useConnection();
  const [method, setMethod] = useState("list_tools");
  const [params, setParams] = useState("{}");
  const [out, setOut] = useState("");
  const [busy, setBusy] = useState(false);

  async function send() {
    setBusy(true);
    setOut("");
    let parsed: unknown = {};
    if (params.trim()) {
      try {
        parsed = JSON.parse(params);
      } catch (e: unknown) {
        setOut("参数不是合法 JSON：" + (e instanceof Error ? e.message : String(e)));
        setBusy(false);
        return;
      }
    }
    try {
      const res = await fetch("/api/rpc", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ method, params: parsed }),
      });
      const json = await res.json();
      setOut(JSON.stringify(json, null, 2));
      if (json?.error?.message) reportError(String(json.error.message));
    } catch (e: unknown) {
      const message = e instanceof Error ? e.message : String(e);
      reportError(message);
      setOut("请求失败：" + message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div>
      <div className="card">
        <div className="card-head">
          <h3>请求</h3>
          <span className="hint">
            method 可填任意工具名或 <code>list_tools</code> / <code>ping</code> / <code>run_goal</code>
          </span>
        </div>
        <label className="field">
          <span>method</span>
          <input value={method} onChange={(e) => setMethod(e.target.value)} />
        </label>
        <label className="field">
          <span>params（JSON）</span>
          <textarea value={params} onChange={(e) => setParams(e.target.value)} />
        </label>
        <Button onClick={send} loading={busy} disabled={!method}>
          发送
        </Button>
        {out && <pre className="output">{out}</pre>}
      </div>
    </div>
  );
}
