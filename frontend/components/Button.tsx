"use client";

import {
  useCallback,
  useRef,
  useState,
  type ButtonHTMLAttributes,
  type CSSProperties,
  type PointerEvent,
  type ReactNode,
} from "react";

/** Pure-CSS spinner (keyframes live in globals.css under `.spinner`). */
export function Spinner({ size = 14 }: { size?: number }) {
  return (
    <span
      className="spinner"
      style={{ width: size, height: size }}
      role="status"
      aria-label="加载中"
    />
  );
}

type Variant = "primary" | "ghost" | "danger";

interface ButtonProps extends Omit<ButtonHTMLAttributes<HTMLButtonElement>, "className"> {
  variant?: Variant;
  loading?: boolean;
  children: ReactNode;
  className?: string;
}

interface Ripple {
  id: number;
  style: CSSProperties;
}

const RIPPLE_MS = 400; // keep in sync with --dur-4 in globals.css

/** True when the OS asks for reduced motion (skip ripple spawning). */
function prefersReducedMotion(): boolean {
  return (
    typeof window !== "undefined" &&
    typeof window.matchMedia === "function" &&
    window.matchMedia("(prefers-reduced-motion: reduce)").matches
  );
}

/**
 * Reusable button. While `loading` is true it shows an inline spinner and is
 * disabled; the label stays visible so layout doesn't jump. A material-style
 * ripple blooms from the press point (skipped under reduced-motion).
 */
export default function Button({
  variant = "primary",
  loading = false,
  disabled,
  children,
  className,
  onPointerDown,
  ...rest
}: ButtonProps) {
  const variantClass = variant === "ghost" ? " ghost" : variant === "danger" ? " danger" : "";
  const [ripples, setRipples] = useState<Ripple[]>([]);
  const nextId = useRef(0);

  const spawnRipple = useCallback(
    (e: PointerEvent<HTMLButtonElement>) => {
      onPointerDown?.(e);
      if (disabled || loading || prefersReducedMotion()) return;
      const el = e.currentTarget;
      const rect = el.getBoundingClientRect();
      // Diameter large enough to cover the whole button from the press point.
      const d = Math.max(rect.width, rect.height) * 1.6;
      const id = nextId.current++;
      const style: CSSProperties = {
        // CSS vars consumed by `.btn .ripple` in globals.css.
        ["--ripple-x" as string]: `${e.clientX - rect.left}px`,
        ["--ripple-y" as string]: `${e.clientY - rect.top}px`,
        ["--ripple-d" as string]: `${d}px`,
      };
      setRipples((cur) => [...cur, { id, style }]);
      window.setTimeout(() => {
        setRipples((cur) => cur.filter((r) => r.id !== id));
      }, RIPPLE_MS);
    },
    [disabled, loading, onPointerDown],
  );

  return (
    <button
      className={`btn${variantClass}${loading ? " is-loading" : ""}${className ? " " + className : ""}`}
      disabled={disabled || loading}
      aria-busy={loading || undefined}
      onPointerDown={spawnRipple}
      {...rest}
    >
      {loading && <Spinner />}
      <span className="btn-label">{children}</span>
      {ripples.map((r) => (
        <span key={r.id} className="ripple" style={r.style} aria-hidden />
      ))}
    </button>
  );
}
