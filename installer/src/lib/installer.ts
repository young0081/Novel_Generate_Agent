// Thin wrappers around the Tauri backend commands.
// All calls are guarded so `npm run build` and a plain browser preview never crash.

import { invoke } from "@tauri-apps/api/core";

export type DetectResult = {
  installed: boolean;
  path: string;
  version: string | null;
};

export type InstallProgress = {
  percent: number;
  message: string;
};

export type InstallReport = {
  shortcuts: number;
  shortcut_errors: string[];
  uninstaller: boolean;
};

function isTauri(): boolean {
  return (
    typeof window !== "undefined" &&
    ("__TAURI_INTERNALS__" in window || "__TAURI__" in window)
  );
}

// Fallback values used only for the in-browser dev preview (no Tauri backend).
const BROWSER_FALLBACK_DIR =
  "C:\\Users\\You\\AppData\\Local\\Programs\\NovelGenerateTeam";
const BROWSER_FALLBACK_VERSION = "0.3.0";

export async function detectExisting(): Promise<DetectResult> {
  if (!isTauri()) {
    return { installed: false, path: BROWSER_FALLBACK_DIR, version: null };
  }
  return await invoke<DetectResult>("detect_existing");
}

export async function defaultDir(): Promise<string> {
  if (!isTauri()) return BROWSER_FALLBACK_DIR;
  return await invoke<string>("default_dir");
}

export async function installerVersion(): Promise<string> {
  if (!isTauri()) return BROWSER_FALLBACK_VERSION;
  return await invoke<string>("installer_version");
}

/**
 * Run the install. Sets up the `install-progress` listener BEFORE invoking,
 * forwarding every event to `onProgress`. The listener is always cleaned up.
 */
export async function install(
  dir: string,
  onProgress: (p: InstallProgress) => void,
): Promise<InstallReport> {
  if (!isTauri()) {
    // Simulated install for browser preview so the UI is fully testable.
    await simulateInstall(onProgress);
    return { shortcuts: 2, shortcut_errors: [], uninstaller: true };
  }

  const { listen } = await import("@tauri-apps/api/event");
  const unlisten = await listen<InstallProgress>("install-progress", (e) => {
    const payload = e.payload as InstallProgress;
    if (payload && typeof payload.percent === "number") {
      onProgress(payload);
    }
  });

  try {
    return await invoke<InstallReport>("install", { dir });
  } finally {
    unlisten();
  }
}

export async function launch(dir: string): Promise<void> {
  if (!isTauri()) {
    console.info("launch (browser preview, no-op)", dir);
    return;
  }
  await invoke<void>("launch", { dir });
}

export async function uninstall(deleteUserData: boolean = false): Promise<string> {
  if (!isTauri()) {
    return "卸载完成（浏览器预览模式）";
  }
  return await invoke<string>("uninstall", { deleteUserData });
}

export async function isUninstallMode(): Promise<boolean> {
  if (!isTauri()) {
    return false;
  }
  return await invoke<boolean>("is_uninstall_mode");
}

// ---- browser-preview simulation -------------------------------------------

const SIM_STEPS: Array<[number, string]> = [
  [6, "正在准备安装环境…"],
  [22, "正在解压程序文件…"],
  [48, "正在写入应用资源…"],
  [70, "正在创建快捷方式…"],
  [88, "正在写入注册表项…"],
  [100, "安装完成"],
];

function delay(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function simulateInstall(
  onProgress: (p: InstallProgress) => void,
): Promise<void> {
  for (const [percent, message] of SIM_STEPS) {
    await delay(620);
    onProgress({ percent, message });
  }
}
