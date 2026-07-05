// Importance indicator rendered as 1..5 cinnabar dots.

interface DotsProps {
  value: number;
  max?: number;
}

export default function Dots({ value, max = 5 }: DotsProps) {
  const v = Math.max(0, Math.min(max, Math.round(value)));
  return (
    <span className="dots" title={`重要度 ${v}/${max}`} aria-label={`重要度 ${v}/${max}`}>
      {Array.from({ length: max }, (_, i) => (
        <span key={i} className={`dots__dot${i < v ? " is-on" : ""}`} />
      ))}
    </span>
  );
}
