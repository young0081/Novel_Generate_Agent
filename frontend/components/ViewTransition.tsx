"use client";

import type { ReactNode } from "react";

/**
 * Lightweight view-transition wrapper. Give it a `viewKey` that changes when the
 * active view changes; React remounts the subtree, replaying the `.view-transition`
 * CSS animation (fade + slight rise). Pure CSS — respects prefers-reduced-motion.
 */
export default function ViewTransition({
  viewKey,
  children,
}: {
  viewKey: string;
  children: ReactNode;
}) {
  return (
    <div className="view-transition" key={viewKey}>
      {children}
    </div>
  );
}
