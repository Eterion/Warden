// Every spinner on one clock. A CSS animation starts when its element appears, so two spinners shown at different
// moments would turn out of step. `v-sync-spin` moves the start of each animation on the element to the document
// timeline's zero, so every spinner shows the same angle at any moment, wherever it is and whenever it appeared. It
// runs once when the element appears or changes (a button turning busy); no timer keeps it in step.
import type { ObjectDirective } from 'vue';

export function sync(el: Element): void {
  if (typeof el.getAnimations !== 'function') return;
  for (const a of el.getAnimations()) if (a.startTime !== 0) a.startTime = 0;
}

export const vSyncSpin: ObjectDirective<Element> = { mounted: sync, updated: sync };

/**
 * Pauses every spinner while the window is out of sight (hidden or minimized; `data-away` on the root, styled in
 * tokens.css) and puts them back on the shared clock when it is in sight again.
 */
export function pauseSpinners(away: boolean): void {
  document.documentElement.toggleAttribute('data-away', away);
  if (!away) syncSpinners();
}

/** Puts every running spinner back on the shared clock (after a pause, or after reduced motion is turned off). */
export function syncSpinners(): void {
  if (typeof document.getAnimations !== 'function') return;
  for (const a of document.getAnimations())
    if ((a as CSSAnimation).animationName === 'spin' && a.startTime !== 0) a.startTime = 0;
}
