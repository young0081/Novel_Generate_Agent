// Bridge to the Electron desktop shell's window controls.
//
// In the Electron build, `desktop/preload.js` injects `window.desktopAPI`. In a
// plain browser it is absent, so `getDesktopAPI()` returns null and the UI hides
// the custom (frameless) title bar.

export interface DesktopAPI {
  isDesktop: boolean;
  minimize: () => void;
  maximizeToggle: () => void;
  close: () => void;
  isMaximized: () => Promise<boolean>;
}

declare global {
  interface Window {
    desktopAPI?: DesktopAPI;
  }
}

export function getDesktopAPI(): DesktopAPI | null {
  if (typeof window === "undefined") return null;
  return window.desktopAPI ?? null;
}
