"use client";

import type { ReactNode } from "react";

/** Friendly placeholder shown when a listing / result set is empty. */
export default function EmptyState({
  icon = "🗂️",
  title,
  hint,
}: {
  icon?: ReactNode;
  title: string;
  hint?: string;
}) {
  return (
    <div className="empty-state">
      <div className="empty-state-icon" aria-hidden>
        {icon}
      </div>
      <div className="empty-state-title">{title}</div>
      {hint && <div className="empty-state-hint">{hint}</div>}
    </div>
  );
}
