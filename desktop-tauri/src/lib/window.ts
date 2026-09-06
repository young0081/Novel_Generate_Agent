// Frameless window controls for the custom title bar.
//
// The window is created with `decorations: false`, so we render our own
// minimize / maximize / close buttons. These are guarded so a browser
// preview (no Tauri runtime) silently no-ops instead of crashing.

import { getCurrentWindow } from "@tauri-apps/api/window";
import { isDesktop } from "./core";
import {
  acquireEditorWriteBarrier,
  cancelPendingEditorRuns,
  flushPendingEditors,
  tryAcquireWorkspaceAction,
} from "./editorPersistence";
import {
  createCloseRequestCoordinator,
  waitForCloseOperation,
} from "./windowClose";

export {
  createCloseRequestCoordinator,
  waitForCloseOperation,
} from "./windowClose";
export type {
  CloseOperationResult,
  CloseRequestCoordinator,
} from "./windowClose";

function win() {
  return getCurrentWindow();
}

const CLOSE_WORKSPACE_WAIT_MS = 1_500;
const CLOSE_WORKSPACE_POLL_MS = 50;
const CLOSE_FLUSH_WAIT_MS = 4_000;

export type WindowCloseState = "idle" | "preparing" | "failed";

let closeState: WindowCloseState = "idle";
const closeStateListeners = new Set<(state: WindowCloseState) => void>();

function publishCloseState(state: WindowCloseState): void {
  closeState = state;
  closeStateListeners.forEach((listener) => listener(state));
}

export function onWindowCloseState(
  listener: (state: WindowCloseState) => void,
): () => void {
  closeStateListeners.add(listener);
  listener(closeState);
  return () => closeStateListeners.delete(listener);
}

async function acquireWorkspaceForClose(): Promise<(() => void) | null> {
  const deadline = performance.now() + CLOSE_WORKSPACE_WAIT_MS;
  for (;;) {
    const release = tryAcquireWorkspaceAction();
    if (release) return release;

    const remaining = deadline - performance.now();
    if (remaining <= 0) return null;
    await new Promise<void>((resolve) => {
      window.setTimeout(resolve, Math.min(CLOSE_WORKSPACE_POLL_MS, remaining));
    });
  }
}

async function prepareClose(): Promise<boolean> {
  let releaseEditorWriteBarrier: (() => void) | null = null;
  let releaseWorkspaceAction: (() => void) | null = null;

  const confirmForcedClose = (reason: string): boolean => {
    try {
      return window.confirm(
        `${reason}\n\n强制关闭可能丢失尚未保存的更改，是否仍要关闭？`,
      );
    } catch (error) {
      console.error("Failed to confirm forced close", error);
      return false;
    }
  };

  try {
    cancelPendingEditorRuns();
    releaseEditorWriteBarrier = acquireEditorWriteBarrier();
    releaseWorkspaceAction = await acquireWorkspaceForClose();
    if (!releaseWorkspaceAction) {
      return confirmForcedClose("后台任务未能及时停止。");
    }

    const flushResult = await waitForCloseOperation(
      Promise.resolve().then(() => flushPendingEditors()),
      CLOSE_FLUSH_WAIT_MS,
    );
    if (flushResult.status === "timed-out") {
      return confirmForcedClose("保存操作未能在 4 秒内完成。");
    }
    if (flushResult.status === "failed") {
      console.error("Failed to flush editors before close", flushResult.error);
      return confirmForcedClose("关闭前保存出现异常。");
    }
    if (!flushResult.value) {
      window.alert("当前编辑文件保存失败，窗口未关闭。请检查错误后重试。");
      return false;
    }
    return true;
  } catch (error) {
    console.error("Failed to prepare window close", error);
    return confirmForcedClose("关闭前准备出现异常。");
  } finally {
    releaseWorkspaceAction?.();
    releaseEditorWriteBarrier?.();
  }
}

const closeCoordinator = createCloseRequestCoordinator(
  async () => {
    publishCloseState("preparing");
    const ready = await prepareClose();
    if (!ready) publishCloseState("idle");
    return ready;
  },
  () => win().destroy(),
  (error) => {
    console.error("Failed to close window", error);
    publishCloseState("failed");
    window.setTimeout(() => {
      if (closeState === "failed") publishCloseState("idle");
    }, 2_400);
  },
);

export async function minimizeWindow(): Promise<void> {
  if (!isDesktop()) return;
  try {
    await win().minimize();
  } catch (error) {
    console.error("Failed to minimize window", error);
  }
}

export async function toggleMaximizeWindow(): Promise<void> {
  if (!isDesktop()) return;
  try {
    await win().toggleMaximize();
  } catch (error) {
    console.error("Failed to toggle window size", error);
  }
}

export async function closeWindow(): Promise<void> {
  if (!isDesktop()) return;
  await closeCoordinator.requestClose();
}

/** Guard OS-level close gestures (Alt+F4 / taskbar close) as well. */
export async function guardWindowClose(): Promise<() => void> {
  if (!isDesktop()) return () => {};
  try {
    return await win().onCloseRequested((event) => {
      // Prevent synchronously so Tauri never waits on a potentially hung save.
      // The coordinator destroys the window once preparation has settled.
      event.preventDefault();
      void closeCoordinator.requestClose();
    });
  } catch (error) {
    console.error("Failed to register window close guard", error);
    return () => {};
  }
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
