"use client";

import { useConnection } from "@/components/Connection";

export default function StatusBar() {
  const { state, ping } = useConnection();

  const label =
    state === "checking" ? "正在连接核心…" : state === "ok" ? "已连接 Rust 核心" : "核心未连接";
  const dotClass = state === "ok" ? "dot ok" : state === "bad" ? "dot bad" : "dot";

  return (
    <button
      type="button"
      className="status"
      onClick={() => void ping()}
      disabled={state === "checking"}
      title="点击重新检测连接"
    >
      <span className={dotClass} />
      {label}
    </button>
  );
}
