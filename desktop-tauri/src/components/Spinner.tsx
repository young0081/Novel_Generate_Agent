// 研墨 — a cinnabar drop circles a quiet ink-stone well.

import { useId } from "react";

interface SpinnerProps {
  size?: number;
}

export function Spinner({ size = 22 }: SpinnerProps) {
  const gradientId = `ink-trail-${useId().replace(/:/g, "")}`;
  return (
    <span
      className="spinner"
      style={{ width: size, height: size }}
      aria-hidden="true"
    >
      <svg width={size} height={size} viewBox="0 0 36 36" className="spinner__svg">
        <defs>
          <linearGradient id={gradientId} x1="0" y1="0" x2="1" y2="1">
            <stop offset="0%" stopColor="var(--spinner-accent)" stopOpacity="0" />
            <stop offset="100%" stopColor="var(--spinner-accent)" stopOpacity="0.92" />
          </linearGradient>
        </defs>
        <circle
          cx="18"
          cy="18"
          r="13"
          fill="none"
          stroke="var(--spinner-ink)"
          strokeWidth="1.8"
          className="spinner__well"
        />
        <path className="spinner__wash" d="M10 21c2.7 4.5 9.5 6.1 14 2.8-2.2 4-8.8 5.5-13.1 1.7-1.4-1.2-2.1-2.8-.9-4.5Z" />
        <g className="spinner__motion">
          <path
            d="M18 5a13 13 0 0 1 11.3 6.5"
            fill="none"
            stroke={`url(#${gradientId})`}
            strokeWidth="2.5"
            strokeLinecap="round"
            className="spinner__arc"
          />
          <circle cx="18" cy="5" r="2.35" fill="var(--spinner-accent)" className="spinner__drop" />
        </g>
      </svg>
    </span>
  );
}

export function LoadingBlock({ label = "正在研墨…" }: { label?: string }) {
  return (
    <div className="loading-block" role="status" aria-live="polite">
      <Spinner size={34} />
      <span className="loading-block__label">{label}</span>
    </div>
  );
}
