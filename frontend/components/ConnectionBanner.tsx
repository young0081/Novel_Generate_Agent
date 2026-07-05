"use client";

import { useEffect, useState } from "react";

import Button from "@/components/Button";
import { useConnection } from "@/components/Connection";

/**
 * Top-of-main banner shown only when the core is unreachable. Offers a "重试"
 * button (re-pings) and can be dismissed; it reappears if a later call fails.
 */
export default function ConnectionBanner() {
  const { state, ping } = useConnection();
  const [dismissed, setDismissed] = useState(false);
  const [retrying, setRetrying] = useState(false);

  // Once we're connected again, clear the dismissed flag so future drops show.
  useEffect(() => {
    if (state === "ok") setDismissed(false);
  }, [state]);

  if (state !== "bad" || dismissed) return null;

  async function retry() {
    setRetrying(true);
    try {
      await ping();
    } finally {
      setRetrying(false);
    }
  }

  return (
    <div className="conn-banner" role="alert">
      <span className="conn-banner-dot" aria-hidden />
      <span className="conn-banner-text">
        无法连接到 Rust 核心。请确认核心已启动，然后点「重试」。
      </span>
      <div className="conn-banner-actions">
        <Button variant="ghost" loading={retrying} onClick={retry}>
          重试
        </Button>
        <button
          type="button"
          className="conn-banner-close"
          onClick={() => setDismissed(true)}
          aria-label="关闭提示"
          title="关闭提示"
        >
          ✕
        </button>
      </div>
    </div>
  );
}
