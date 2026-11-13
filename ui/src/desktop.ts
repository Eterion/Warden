// The few things the page asks of the desktop app's window (Tauri): its controls, the system folder picker and Quit.
// Everything else still goes to the local server over plain web requests. In a browser none of this exists and
// `desktop.inApp` is false.

import { invoke, isTauri } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { reactive } from 'vue';

export const desktop = reactive({
  /** Running inside the desktop app's window. */
  inApp: isTauri(),
  /** The window fills the screen (the middle control then restores it). */
  maximized: false,
  /** Minimized to the taskbar. The page is not hidden then, so updates pause on this too. */
  minimized: false,
});

/** Follows the window's state: maximized for the middle control, minimized to pause updates. */
export function startDesktop(): void {
  if (!desktop.inApp) return;
  const win = getCurrentWindow();
  const read = () =>
    void Promise.all([win.isMaximized(), win.isMinimized()]).then(
      ([max, min]) => {
        desktop.maximized = max;
        desktop.minimized = min;
      },
      () => undefined,
    );
  read();
  void win.onResized(read);
}

export function minimize(): void {
  void getCurrentWindow().minimize();
}

export function toggleMaximize(): void {
  void getCurrentWindow().toggleMaximize();
}

/** Closes the window: Warden quits, or stays in the tray when Keep running in the tray is on. */
export function closeWindow(): void {
  void getCurrentWindow().close();
}

/** Quits Warden, tray and core included. */
export function quit(): void {
  void invoke('quit');
}

/** The system folder picker. `null` when cancelled. */
export function pickFolder(): Promise<string | null> {
  return invoke<string | null>('pick_folder');
}
