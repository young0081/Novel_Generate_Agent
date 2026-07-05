// 朱砂印章 — a cinnabar name-seal used as the brand mark.
// Rendered as a carved stamp: a cinnabar block with an inset frame (边框)
// and the character cut in relief. Subtle mottling reads as ink on paper.

interface SealProps {
  size?: number;
  char?: string;
  /** Slightly stronger presence for hero placements (modal). */
  tone?: "brand" | "soft";
}

export default function Seal({ size = 26, char = "墨", tone = "brand" }: SealProps) {
  // proportional carving: frame thickness + radius scale with the block
  const frame = Math.max(1.5, size * 0.05);
  const radius = Math.max(4, size * 0.16);
  return (
    <span
      className={`seal seal--${tone}`}
      style={
        {
          width: size,
          height: size,
          fontSize: Math.round(size * 0.58),
          borderRadius: radius,
          "--seal-frame": `${frame}px`,
          "--seal-radius": `${radius}px`,
        } as React.CSSProperties
      }
      aria-hidden="true"
    >
      <span className="seal__char">{char}</span>
    </span>
  );
}
