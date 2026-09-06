const FOLLOW_DISTANCE_PX = 180;

function scrollParentFor(node: HTMLElement): HTMLElement | null {
  let parent = node.parentElement;
  while (parent) {
    const style = window.getComputedStyle(parent);
    if (
      /(auto|scroll)/.test(style.overflowY) &&
      parent.scrollHeight > parent.clientHeight + 1
    ) {
      return parent;
    }
    parent = parent.parentElement;
  }
  return null;
}

/** Keep a live transcript pinned only while the reader is already near its end. */
export function scrollLiveAnchor(
  anchor: HTMLElement | null,
  options: { live: boolean; force?: boolean },
): void {
  if (!anchor) return;
  const scroller = scrollParentFor(anchor);
  if (scroller && !options.force) {
    const distance = scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight;
    if (distance > FOLLOW_DISTANCE_PX) return;
  }
  const reduced = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  anchor.scrollIntoView({
    behavior: options.live || reduced ? "auto" : "smooth",
    block: "end",
  });
}
