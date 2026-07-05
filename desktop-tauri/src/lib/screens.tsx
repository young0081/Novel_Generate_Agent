// Navigation model shared between the sidebar and the router shell.

import type { ReactNode } from "react";
import {
  IconCompass,
  IconChat,
  IconBrush,
  IconHistory,
  IconScroll,
  IconUser,
  IconThread,
  IconMountain,
  IconBranch,
  IconClock,
  IconTools,
  IconProviders,
} from "../components/icons";

export type ScreenId =
  | "planning"
  | "chat"
  | "studio"
  | "sessions"
  | "chapters"
  | "characters"
  | "foreshadow"
  | "settings"
  | "collab"
  | "checkpoints"
  | "tools"
  | "providers";

export interface ScreenDef {
  id: ScreenId;
  label: string;
  hint: string;
  icon: (props: { size?: number }) => ReactNode;
}

export const SCREENS: ScreenDef[] = [
  { id: "planning", label: "策划", hint: "plan", icon: IconCompass },
  { id: "chat", label: "探讨", hint: "discuss", icon: IconChat },
  { id: "studio", label: "创作", hint: "write", icon: IconBrush },
  { id: "sessions", label: "会话", hint: "history", icon: IconHistory },
  { id: "chapters", label: "章节", hint: "book", icon: IconScroll },
  { id: "characters", label: "人物", hint: "character", icon: IconUser },
  { id: "foreshadow", label: "伏笔", hint: "foreshadow", icon: IconThread },
  { id: "settings", label: "设定", hint: "world", icon: IconMountain },
  { id: "collab", label: "协作", hint: "version", icon: IconBranch },
  { id: "checkpoints", label: "快照", hint: "snapshot", icon: IconClock },
  { id: "tools", label: "工具", hint: "tools", icon: IconTools },
  { id: "providers", label: "供应商", hint: "providers", icon: IconProviders },
];
