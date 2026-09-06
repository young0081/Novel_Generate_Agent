import { useEffect, useRef, useState, type RefObject } from "react";

const DIALOG_LAYER_SELECTOR =
  '[role="dialog"][aria-modal="true"], [role="alertdialog"][aria-modal="true"]';

/** Return the foremost mounted modal layer in document order. */
export function topDialogLayer(scope: ParentNode = document): HTMLElement | null {
  const layers = scope.querySelectorAll<HTMLElement>(DIALOG_LAYER_SELECTOR);
  return layers.length > 0 ? layers[layers.length - 1] : null;
}

/** Keyboard dismissal and focus trapping belong only to the foremost layer. */
export function isTopDialogLayer(
  layer: HTMLElement | null,
  scope: ParentNode = document,
): layer is HTMLElement {
  return layer !== null && topDialogLayer(scope) === layer;
}

export function layerExitDelay(ms: number): number {
  if (typeof window === "undefined" || typeof window.matchMedia !== "function") return ms;
  return window.matchMedia("(prefers-reduced-motion: reduce)").matches ? 0 : ms;
}

export function useLayerPresence(open: boolean, exitMs = 220): {
  mounted: boolean;
  closing: boolean;
} {
  const [mounted, setMounted] = useState(open);
  const [closing, setClosing] = useState(false);

  useEffect(() => {
    if (open) {
      setMounted(true);
      setClosing(false);
      return;
    }
    if (!mounted) return;
    setClosing(true);
    const timer = window.setTimeout(() => {
      setMounted(false);
      setClosing(false);
    }, layerExitDelay(exitMs));
    return () => window.clearTimeout(timer);
  }, [exitMs, mounted, open]);

  return { mounted, closing };
}

export function useDialogFocus(
  open: boolean,
  rootRef: RefObject<HTMLElement | null>,
  onClose: () => void,
  blocked = false,
): void {
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;

  useEffect(() => {
    if (!open || blocked) return;
    const previouslyFocused = document.activeElement as HTMLElement | null;
    const frame = window.requestAnimationFrame(() => {
      const root = rootRef.current;
      if (!isTopDialogLayer(root)) return;
      const target = root?.querySelector<HTMLElement>("[data-autofocus]") ?? root;
      target?.focus();
    });
    const onKeyDown = (event: KeyboardEvent) => {
      const root = rootRef.current;
      if (!isTopDialogLayer(root)) return;
      if (event.key === "Escape") {
        event.preventDefault();
        onCloseRef.current();
        return;
      }
      if (event.key !== "Tab") return;
      const focusable = Array.from(root.querySelectorAll<HTMLElement>(
        'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])',
      ));
      if (focusable.length === 0) {
        event.preventDefault();
        root.focus();
        return;
      }
      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      if (event.shiftKey && (document.activeElement === first || !root.contains(document.activeElement))) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.cancelAnimationFrame(frame);
      window.removeEventListener("keydown", onKeyDown);
      if (previouslyFocused?.isConnected) previouslyFocused.focus();
    };
  }, [blocked, open, rootRef]);
}
