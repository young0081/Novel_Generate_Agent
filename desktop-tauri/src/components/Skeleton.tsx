// Loading placeholders that mirror the real layouts while content streams in.
// Each block sweeps a faint paper-light across itself (.skeleton in app.css),
// which collapses to a still frame under prefers-reduced-motion.

/** A single shimmering bar. Width/height are inline so callers can shape it. */
export function SkeletonBar({
  width = "100%",
  height = 12,
  radius,
  style,
}: {
  width?: number | string;
  height?: number | string;
  radius?: number | string;
  style?: React.CSSProperties;
}) {
  return (
    <span
      className="skeleton"
      style={{
        display: "block",
        width,
        height,
        borderRadius: radius,
        ...style,
      }}
      aria-hidden="true"
    />
  );
}

/** A paper-slip-shaped skeleton card. */
function SkeletonCard() {
  return (
    <div className="skel-card" aria-hidden="true">
      <div className="skel-card__row">
        <SkeletonBar width="58%" height={16} radius={6} />
        <SkeletonBar width={46} height={18} radius={999} />
      </div>
      <SkeletonBar width="100%" height={11} radius={5} />
      <SkeletonBar width="92%" height={11} radius={5} />
      <SkeletonBar width="74%" height={11} radius={5} />
      <div className="skel-card__foot">
        <SkeletonBar width={64} height={10} radius={999} />
        <SkeletonBar width={40} height={10} radius={999} />
      </div>
    </div>
  );
}

/**
 * A grid of skeleton cards used while a card screen (tools / memory /
 * providers) loads, so the panel keeps its shape instead of collapsing to a
 * lone spinner.
 */
export function SkeletonGrid({ count = 6 }: { count?: number }) {
  return (
    <div className="slip-grid" role="status" aria-label="正在加载">
      {Array.from({ length: count }, (_, i) => (
        <SkeletonCard key={i} />
      ))}
    </div>
  );
}
