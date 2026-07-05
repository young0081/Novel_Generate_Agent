// Inline SVG icons — no icon library, offline-safe.
// All icons inherit `currentColor` and accept a `size` prop.

import type { SVGProps } from "react";

interface IconProps extends SVGProps<SVGSVGElement> {
  size?: number;
}

function base({ size = 18, ...rest }: IconProps) {
  return {
    width: size,
    height: size,
    viewBox: "0 0 24 24",
    fill: "none",
    stroke: "currentColor",
    strokeWidth: 1.6,
    strokeLinecap: "round" as const,
    strokeLinejoin: "round" as const,
    ...rest,
  };
}

export const IconScroll = (p: IconProps) => (
  <svg {...base(p)}>
    <path d="M5 4h11a2 2 0 0 1 2 2v12a2 2 0 0 0 2 2H8a2 2 0 0 1-2-2V6a2 2 0 0 0-2-2Z" />
    <path d="M9 8h6M9 12h6M9 16h3" />
  </svg>
);

export const IconUser = (p: IconProps) => (
  <svg {...base(p)}>
    <circle cx="12" cy="8" r="3.4" />
    <path d="M5.5 19a6.5 6.5 0 0 1 13 0" />
  </svg>
);

export const IconThread = (p: IconProps) => (
  // 伏笔 — a knotted thread / foreshadowing motif
  <svg {...base(p)}>
    <path d="M6 4c0 4 12 4 12 8s-12 4-12 8" />
    <circle cx="6" cy="4" r="1.4" />
    <circle cx="18" cy="20" r="1.4" />
  </svg>
);

export const IconMountain = (p: IconProps) => (
  // 设定 — landscape / worldbuilding
  <svg {...base(p)}>
    <path d="M3 18l5-7 3 4 3-5 7 8Z" />
    <circle cx="17" cy="6.5" r="1.6" />
  </svg>
);

export const IconClock = (p: IconProps) => (
  <svg {...base(p)}>
    <circle cx="12" cy="12" r="8" />
    <path d="M12 8v4l3 2" />
  </svg>
);

export const IconTools = (p: IconProps) => (
  <svg {...base(p)}>
    <path d="M14.5 6a3.5 3.5 0 0 0-4.9 4.2l-5 5a1.5 1.5 0 0 0 2.1 2.1l5-5A3.5 3.5 0 0 0 18 7.5L15.5 10 14 8.5 16.5 6Z" />
  </svg>
);

export const IconProviders = (p: IconProps) => (
  // 供应商 — interlinked nodes / a network of model endpoints
  <svg {...base(p)}>
    <circle cx="6" cy="6" r="2.4" />
    <circle cx="18" cy="6" r="2.4" />
    <circle cx="12" cy="18" r="2.4" />
    <path d="M7.7 7.7 10.6 16M16.3 7.7 13.4 16M8.2 6h7.6" />
  </svg>
);

export const IconCompass = (p: IconProps) => (
  // 策划 — a drafting compass: two legs hinged at a pivot over a baseline,
  // the act of planning/blueprinting before the first brushstroke.
  <svg {...base(p)}>
    <circle cx="12" cy="5" r="1.7" />
    <path d="M11.3 6.5 6 19M12.7 6.5 18 19" />
    <path d="M8.7 14.2h6.6" />
    <path d="M6 19h.01M18 19h.01" />
  </svg>
);

export const IconChat = (p: IconProps) => (
  // 探讨 — two overlapping speech bubbles, a back-and-forth discussion.
  <svg {...base(p)}>
    <path d="M3 6.5a2 2 0 0 1 2-2h8a2 2 0 0 1 2 2v4a2 2 0 0 1-2 2H7l-4 3.2V6.5Z" />
    <path d="M9 14.6V15a2 2 0 0 0 2 2h6l3 2.4V11a2 2 0 0 0-1.6-2" />
  </svg>
);

export const IconSeed = (p: IconProps) => (
  // 构思 — a sprouting seed/idea: a stem rising from a seed with two leaves.
  <svg {...base(p)}>
    <path d="M12 21v-7" />
    <path d="M12 14c0-2.6-2-4.4-4.6-4.6C7.2 12 9.2 14 12 14Z" />
    <path d="M12 12c0-2.6 2-4.6 4.6-4.8C16.8 10 14.8 12 12 12Z" />
    <circle cx="12" cy="5.4" r="1.6" />
  </svg>
);

export const IconBrush = (p: IconProps) => (
  // 创作 — a writing brush (毛笔): a slanted handle ending in a loaded tip
  <svg {...base(p)}>
    <path d="M20 4 9.5 14.5" />
    <path d="M6 13c2.2-.5 4 .4 4.8 1.2.8.8 1.7 2.6 1.2 4.8-2 .5-3.8.2-5-1s-1.5-3-1-5Z" />
    <path d="M6 13c-1.4.4-2.3 1.4-2.8 3.2C2.8 18 3 19 3 19s1 .2 2.8-.2C7.6 18.3 8.6 17.4 9 16" />
  </svg>
);

export const IconBranch = (p: IconProps) => (
  // 协作/版本 — a git-style branch: a trunk forking to a side line
  <svg {...base(p)}>
    <circle cx="6" cy="5" r="2.2" />
    <circle cx="6" cy="19" r="2.2" />
    <circle cx="18" cy="8" r="2.2" />
    <path d="M6 7.2v9.6M6 12h6a4 4 0 0 0 4-4" />
  </svg>
);

export const IconCommit = (p: IconProps) => (
  // a commit node on a line
  <svg {...base(p)}>
    <circle cx="12" cy="12" r="3.2" />
    <path d="M3 12h5.8M15.2 12H21" />
  </svg>
);

export const IconDiff = (p: IconProps) => (
  // 对比 — a small plus over minus, the diff sigil
  <svg {...base(p)}>
    <path d="M6 5v6M3 8h6" />
    <path d="M15 16h6" />
    <path d="M5 19h14" />
  </svg>
);

export const IconThought = (p: IconProps) => (
  // 模型思考 — a speech/thought bubble for the assistant's reasoning
  <svg {...base(p)}>
    <path d="M4 6a2 2 0 0 1 2-2h12a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H9l-4 4v-4a2 2 0 0 1-1-2V6Z" />
    <path d="M8.5 9h7M8.5 12h4" />
  </svg>
);

export const IconObserve = (p: IconProps) => (
  // tool observation — a magnifier over a small mark
  <svg {...base(p)}>
    <circle cx="10.5" cy="10.5" r="5.5" />
    <path d="m20 20-4.4-4.4M8.5 10.5h4M10.5 8.5v4" />
  </svg>
);

export const IconUsers = (p: IconProps) => (
  // 协作 — two figures, async teamwork
  <svg {...base(p)}>
    <circle cx="9" cy="8" r="3" />
    <path d="M3.5 19a5.5 5.5 0 0 1 11 0" />
    <path d="M16 5.2a3 3 0 0 1 0 5.6M17.5 13.4A5.5 5.5 0 0 1 20.5 18.5" />
  </svg>
);

export const IconKey = (p: IconProps) => (
  <svg {...base(p)}>
    <circle cx="8" cy="8" r="3.4" />
    <path d="m10.4 10.4 8 8M16 16l2-2M14 18l1.5-1.5" />
  </svg>
);

export const IconEye = (p: IconProps) => (
  <svg {...base(p)}>
    <path d="M2.5 12S6 5.5 12 5.5 21.5 12 21.5 12 18 18.5 12 18.5 2.5 12 2.5 12Z" />
    <circle cx="12" cy="12" r="2.6" />
  </svg>
);

export const IconEyeOff = (p: IconProps) => (
  <svg {...base(p)}>
    <path d="M4 4l16 16" />
    <path d="M9.6 5.8A9.6 9.6 0 0 1 12 5.5c6 0 9.5 6.5 9.5 6.5a16 16 0 0 1-3 3.6M6.3 7.9A16 16 0 0 0 2.5 12S6 18.5 12 18.5a9.4 9.4 0 0 0 3.3-.6" />
    <path d="M9.9 9.9a2.6 2.6 0 0 0 3.6 3.6" />
  </svg>
);

export const IconPlug = (p: IconProps) => (
  // 测试连接 — a power plug, for "connect / test"
  <svg {...base(p)}>
    <path d="M9 3v4M15 3v4" />
    <path d="M7 7h10v3a5 5 0 0 1-10 0V7Z" />
    <path d="M12 15v6" />
  </svg>
);

export const IconTrash = (p: IconProps) => (
  <svg {...base(p)}>
    <path d="M4 7h16M9 7V5a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v2" />
    <path d="M6 7l1 12a1 1 0 0 0 1 1h8a1 1 0 0 0 1-1l1-12M10 11v6M14 11v6" />
  </svg>
);

export const IconPencil = (p: IconProps) => (
  <svg {...base(p)}>
    <path d="M4 20h4l10-10-4-4L4 16v4Z" />
    <path d="M13.5 6.5l4 4" />
  </svg>
);

export const IconStar = (p: IconProps) => (
  // 默认/当前 marker — a small five-point star
  <svg {...base(p)}>
    <path d="M12 4.5l2.2 4.6 5 .7-3.6 3.5.9 5-4.5-2.4L7.5 18.3l.9-5L4.8 9.8l5-.7L12 4.5Z" />
  </svg>
);

export const IconPlus = (p: IconProps) => (
  <svg {...base(p)}>
    <path d="M12 5v14M5 12h14" />
  </svg>
);

export const IconSave = (p: IconProps) => (
  <svg {...base(p)}>
    <path d="M5 5h11l3 3v11a1 1 0 0 1-1 1H6a1 1 0 0 1-1-1V5Z" />
    <path d="M8 5v4h6V5M8 19v-5h8v5" />
  </svg>
);

export const IconSearch = (p: IconProps) => (
  <svg {...base(p)}>
    <circle cx="11" cy="11" r="6" />
    <path d="m20 20-3.2-3.2" />
  </svg>
);

export const IconRefresh = (p: IconProps) => (
  <svg {...base(p)}>
    <path d="M20 11a8 8 0 0 0-14.3-4.3M4 13a8 8 0 0 0 14.3 4.3" />
    <path d="M4 4v3.7h3.7M20 20v-3.7h-3.7" />
  </svg>
);

export const IconFolder = (p: IconProps) => (
  <svg {...base(p)}>
    <path d="M3 7a1 1 0 0 1 1-1h5l2 2h8a1 1 0 0 1 1 1v8a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1Z" />
  </svg>
);

export const IconFile = (p: IconProps) => (
  <svg {...base(p)}>
    <path d="M7 3h7l4 4v13a1 1 0 0 1-1 1H7a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1Z" />
    <path d="M14 3v4h4" />
  </svg>
);

export const IconCheck = (p: IconProps) => (
  <svg {...base(p)}>
    <path d="m5 13 4 4L19 7" />
  </svg>
);

export const IconRestore = (p: IconProps) => (
  <svg {...base(p)}>
    <path d="M3 12a9 9 0 1 0 3-6.7" />
    <path d="M3 4v4h4" />
  </svg>
);

export const IconClose = (p: IconProps) => (
  <svg {...base(p)}>
    <path d="M6 6l12 12M18 6 6 18" />
  </svg>
);

export const IconInfo = (p: IconProps) => (
  <svg {...base(p)}>
    <circle cx="12" cy="12" r="8.5" />
    <path d="M12 11v5M12 8h.01" />
  </svg>
);

export const IconWarn = (p: IconProps) => (
  <svg {...base(p)}>
    <path d="M12 4 2.5 20h19L12 4Z" />
    <path d="M12 10v4M12 17h.01" />
  </svg>
);

export const IconTag = (p: IconProps) => (
  <svg {...base(p)}>
    <path d="M3 12V5a2 2 0 0 1 2-2h7l9 9-9 9-9-9Z" />
    <circle cx="8" cy="8" r="1.3" />
  </svg>
);

export const IconArchive = (p: IconProps) => (
  <svg {...base(p)}>
    <path d="M4 7h16M5 7l1 12a1 1 0 0 0 1 1h10a1 1 0 0 0 1-1l1-12" />
    <path d="M4 7l1.4-2.6A1 1 0 0 1 6.3 4h11.4a1 1 0 0 1 .9.4L20 7M10 11h4" />
  </svg>
);

export const IconChevron = (p: IconProps) => (
  // a small downward chevron — expand/collapse affordance
  <svg {...base(p)}>
    <path d="M6 9l6 6 6-6" />
  </svg>
);

export const IconHistory = (p: IconProps) => (
  // 会话历史 — a clock with a counter-clockwise "back in time" arrow
  <svg {...base(p)}>
    <path d="M3.2 12a8.8 8.8 0 1 0 2.6-6.2" />
    <path d="M3 4v3.6h3.6" />
    <path d="M12 8.2v4l3 1.8" />
  </svg>
);

export const IconArrowRight = (p: IconProps) => (
  <svg {...base(p)}>
    <path d="M5 12h14M12 5l7 7-7 7" />
  </svg>
);

/** 推演 / World Simulator — branching futures */
export const IconSimulate = (p: IconProps) => (
  <svg {...base(p)}>
    <circle cx="6" cy="6" r="2" />
    <circle cx="6" cy="18" r="2" />
    <circle cx="18" cy="12" r="2" />
    <path d="M8 6.8l7.5 4.5M8 17.2l7.5-4.5" />
  </svg>
);

/* window controls — hairline, optically centered in a 12-box */
export const WinMin = () => (
  <svg width="12" height="12" viewBox="0 0 12 12" stroke="currentColor" fill="none">
    <line x1="2.5" y1="6.5" x2="9.5" y2="6.5" strokeWidth="1.1" strokeLinecap="round" />
  </svg>
);
export const WinMax = () => (
  <svg width="12" height="12" viewBox="0 0 12 12" stroke="currentColor" fill="none">
    <rect x="2.4" y="2.4" width="7.2" height="7.2" rx="1.4" strokeWidth="1.1" />
  </svg>
);
export const WinRestore = () => (
  <svg width="12" height="12" viewBox="0 0 12 12" stroke="currentColor" fill="none">
    <rect x="2.2" y="3.6" width="6" height="6" rx="1.2" strokeWidth="1.1" />
    <path d="M4.4 3.6V3a1 1 0 0 1 1-1h3.4a1 1 0 0 1 1 1v3.4a1 1 0 0 1-1 1h-.6" strokeWidth="1.1" strokeLinecap="round" />
  </svg>
);
export const WinCloseIcon = () => (
  <svg width="12" height="12" viewBox="0 0 12 12" stroke="currentColor" fill="none">
    <line x1="2.8" y1="2.8" x2="9.2" y2="9.2" strokeWidth="1.1" strokeLinecap="round" />
    <line x1="9.2" y1="2.8" x2="2.8" y2="9.2" strokeWidth="1.1" strokeLinecap="round" />
  </svg>
);

/* An elegant single ink stroke (一笔) — tapered, calligraphic, no scribble.
   Drawn as a filled closed shape: thin at the dry tail, swelling in the
   pressured middle, lifting to a fine point. Used as a faint corner flourish. */
export const BrushStroke = (props: SVGProps<SVGSVGElement>) => (
  <svg viewBox="0 0 240 130" fill="currentColor" {...props}>
    <path d="M11 86c18-13 39-26 64-37 28-12 60-22 96-25 18-1.5 35-1 48 3-14-.6-30 .2-46 2.4-34 4.6-65 14.6-92 26.5C58 67 35 78 18 89c-3 2-6 .5-7-1.4-.5-1 .1-1.6 0-1.6Z" />
    <path d="M168 30c14 1 27 4 36 10-11-3-24-4.4-37-4.6-3 0-3.6-5.1 1-5.4Z" opacity="0.55" />
  </svg>
);

/* A small lone ink stroke for empty states — one calm tapered sweep. */
export const BrushMark = (props: SVGProps<SVGSVGElement>) => (
  <svg width="72" height="56" viewBox="0 0 72 56" fill="currentColor" {...props}>
    <path d="M7 40c10-8 22-16 36-21 12-4.4 24-7 35-6.4 5 .3 9 1.3 12 3.2-4-.9-9-1.2-14-.9-15 .9-30 5.6-43 11.6C20 35 12 41 8 45c-2 1.6-4 .4-4.4-1.2-.3-1.2.4-2 3.4-3.8Z" />
    <circle cx="58" cy="14" r="3.2" opacity="0.45" />
  </svg>
);

/* The taper used under section titles — a brush-loaded line that thins to nil. */
export const TitleStroke = (props: SVGProps<SVGSVGElement>) => (
  <svg viewBox="0 0 120 8" fill="currentColor" preserveAspectRatio="none" {...props}>
    <path d="M0 4.2c10-1.6 22-2.4 40-2.6 26-.3 52 .4 80 2.1-26 .9-52 1-78 .7C20 6.1 9 5.4 0 4.2Z" />
  </svg>
);
