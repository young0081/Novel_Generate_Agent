// Frameless window controls for the custom title bar.
//
// The window is created with `decorations: false`, so we render our own
// minimize / maximize / close buttons. These are guarded so a browser
// preview (no Tauri runtime) silently no-ops instead of crashing.

import { getCurrentWindow } from "@tauri-apps/api/window";
import { isDesktop } from "./core";

function win() {
  return getCurrentWindow();
}

export async function minimizeWindow(): Promise<void> {
  if (!isDesktop()) return;
  await win().minimize();
}

export async function toggleMaximizeWindow(): Promise<void> {
  if (!isDesktop()) return;
  await win().toggleMaximize();
}

export async function closeWindow(): Promise<void> {
  if (!isDesktop()) return;
  await win().close();
}

export async function isWindowMaximized(): Promise<boolean> {
  if (!isDesktop()) return false;
  try {
    return await win().isMaximized();
  } catch {
    return false;
  }
}

/**
 * Subscribe to window resize events (used to keep the maximize/restore icon
 * in sync). Returns an unlisten function. No-ops safely in a browser.
 */
export async function onWindowResized(
  cb: () => void,
): Promise<() => void> {
  if (!isDesktop()) return () => {};
  try {
    return await win().onResized(() => cb());
  } catch {
    return () => {};
  }
}
