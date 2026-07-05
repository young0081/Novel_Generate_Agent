// Frameless window controls (minimize / close).
// Guarded so the app still works in a plain browser preview (no Tauri globals).

type AppWindow = {
  minimize: () => Promise<void>;
  close: () => Promise<void>;
};

let cached: AppWindow | null = null;

function isTauri(): boolean {
  return (
    typeof window !== "undefined" &&
    // Tauri v2 injects __TAURI_INTERNALS__ into the webview global.
    ("__TAURI_INTERNALS__" in window || "__TAURI__" in window)
  );
}

async function getWindow(): Promise<AppWindow | null> {
  if (cached) return cached;
  if (!isTauri()) return null;
  try {
    const mod = await import("@tauri-apps/api/window");
    cached = mod.getCurrentWindow();
    return cached;
  } catch {
    return null;
  }
}

export async function minimizeWindow(): Promise<void> {
  try {
    const w = await getWindow();
    if (w) await w.minimize();
  } catch (err) {
    // Never let a window control crash the UI.
    console.warn("minimizeWindow failed", err);
  }
}

export async function closeWindow(): Promise<void> {
  try {
    const w = await getWindow();
    if (w) await w.close();
  } catch (err) {
    console.warn("closeWindow failed", err);
  }
}
