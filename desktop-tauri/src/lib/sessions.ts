// Typed client for the session library (persisted 创作 / 探讨 sessions).
//
// Like the other lib clients, every call is guarded so running outside the
// desktop shell yields a friendly Chinese message instead of an opaque crash.

import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { isDesktop, NotInDesktopError } from "./core";
import type { Session } from "./studio";

/** What kind of session a record is. */
export type SessionKind = "writing" | "discuss" | "planning" | string;

/** A lightweight summary for list views. */
export interface SessionSummary {
  id: string;
  title: string;
  kind: SessionKind;
  goal: string | null;
  messages: number;
  created_ms: number;
  updated_ms: number;
  preview: string;
}

/** A full session record (the conversation + metadata) for resuming. */
export interface SessionRecord {
  session: Session;
  kind: SessionKind;
  goal: string | null;
}

async function invoke<T>(
  cmd: string,
  args?: Record<string, unknown>,
): Promise<T> {
  if (!isDesktop()) {
    throw new NotInDesktopError();
  }
  return (await tauriInvoke(cmd, args)) as T;
}

/** All persisted sessions, newest first. */
export async function listSessions(): Promise<SessionSummary[]> {
  return invoke<SessionSummary[]>("sessions_list");
}

/** Load one full session record by id. */
export async function getSession(id: string): Promise<SessionRecord> {
  return invoke<SessionRecord>("session_get", { id });
}

/** Delete a session by id; resolves with the updated list. */
export async function deleteSession(id: string): Promise<SessionSummary[]> {
  return invoke<SessionSummary[]>("session_delete", { id });
}

/** Human label for a session kind. */
export const KIND_LABEL: Record<string, string> = {
  writing: "创作",
  discuss: "探讨",
  planning: "策划",
};

/** Format an epoch-millis timestamp as a compact local date-time. */
export function formatTime(ms: number): string {
  if (!ms) return "";
  try {
    const d = new Date(ms);
    const pad = (n: number) => String(n).padStart(2, "0");
    return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(
      d.getHours(),
    )}:${pad(d.getMinutes())}`;
  } catch {
    return "";
  }
}
