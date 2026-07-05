// 研墨 — an ink-drop spinner. A cinnabar drop orbits a faint ink ring and
// trails a tapered wash, evoking ink dispersing in water. Pure CSS/SVG,
// offline-safe, and it honours prefers-reduced-motion via the stylesheet.

interface SpinnerProps {
  size?: number;
}

export function Spinner({ size = 22 }: SpinnerProps) {
  return (
    <span
      className="spinner"
      style={{ width: size, height: size }}
      aria-hidden="true"
    >
      <svg width={size} height={size} viewBox="0 0 36 36" className="spinner__svg">
        <defs>
          <linearGradient id="ink-trail" x1="0" y1="0" x2="1" y2="1">
            <stop offset="0%" stopColor="var(--cinnabar-bright)" stopOpacity="0" />
            <stop offset="100%" stopColor="var(--cinnabar)" stopOpacity="0.9" />
          </linearGradient>
        </defs>
        {/* faint ground ring — the ink-stone well */}
        <circle
          cx="18"
          cy="18"
          r="13"
          fill="none"
          stroke="var(--line-strong)"
          strokeWidth="2"
          opacity="0.55"
        />
        {/* the dispersing arc */}
        <path
          d="M18 5a13 13 0 0 1 11.3 6.5"
          fill="none"
          stroke="url(#ink-trail)"
          strokeWidth="2.4"
          strokeLinecap="round"
          className="spinner__arc"
        />
        {/* the ink drop */}
        <circle cx="18" cy="5" r="2.4" fill="var(--cinnabar)" className="spinner__drop" />
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
