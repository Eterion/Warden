// The app's state, kept in step with the server. Updates arrive through one long poll while the window is visible
// (the server answers when something changes); nothing runs while it is hidden or minimized.

import { useDocumentVisibility, useEventListener, useIntervalFn, usePreferredReducedMotion } from '@vueuse/core';
import { effectScope, reactive, watch } from 'vue';
import { ApiError, api, type Account, type State, type Theme } from './api';
import { onAnimationStart, pauseSpinners, syncSpinners } from './components/spin';
import { desktop } from './desktop';

export const store = reactive({
  state: null as State | null,
  /** False while the server cannot be reached. */
  connected: true,
  /** The clock the relative times use; ticks while visible. */
  now: Date.now(),
  refreshing: false,
  /** Out of sight: the page hidden or the desktop app's window minimized. Updates pause while it is. */
  away: false,
});

let live = false;
let controller: AbortController | null = null;
const RETRY_MS = 5000;
const CLOCK_MS = 30_000;

function applyTheme(theme: Theme): void {
  const root = document.documentElement;
  if (theme === 'system') delete root.dataset.theme;
  else root.dataset.theme = theme;
}

/** Takes in a new state from the server. */
export function apply(next: State): void {
  store.state = next;
  store.connected = true;
  applyTheme(next.settings.theme);
}

async function load(): Promise<void> {
  try {
    apply(await api.state());
  } catch {
    store.connected = false;
  }
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

/** Out of sight: a hidden page, or the desktop app's window minimized. Set up by startStore. */
let away = (): boolean => false;

async function follow(): Promise<void> {
  if (live) return;
  live = true;
  try {
    while (!away()) {
      controller = new AbortController();
      try {
        apply(await api.stateAfter(store.state?.revision ?? -1, controller.signal));
      } catch (e) {
        if (controller.signal.aborted) return;
        if (!(e instanceof ApiError)) store.connected = false;
        await sleep(RETRY_MS);
        if (!away()) await load();
      }
    }
  } finally {
    live = false;
  }
}

/** Starts following the server. */
export function startStore(): void {
  // The app's whole life: one scope, never stopped.
  effectScope(true).run(() => {
    const visibility = useDocumentVisibility();
    away = () => visibility.value === 'hidden' || desktop.minimized;
    const clock = useIntervalFn(() => (store.now = Date.now()), CLOCK_MS, { immediate: false });

    watch(
      away,
      (hidden) => {
        store.away = hidden;
        pauseSpinners(hidden);
        if (hidden) {
          controller?.abort();
          clock.pause();
          return;
        }
        store.now = Date.now();
        clock.resume();
        void load().then(follow);
      },
      { immediate: true },
    );

    // Turning reduced motion off starts the spinners' animations afresh: put them back on the shared clock once the
    // page has restyled.
    watch(usePreferredReducedMotion(), () => requestAnimationFrame(syncSpinners));
    // A spinner drawn again or moved starts its animation afresh: back onto the shared clock.
    useEventListener(document, 'animationstart', onAnimationStart);
  });
}

/** Runs an action that answers with the new state. Returns the server's error sentence, or null. */
export async function act(run: () => Promise<State>): Promise<string | null> {
  try {
    apply(await run());
    return null;
  } catch (e) {
    if (e instanceof ApiError) return e.message;
    store.connected = false;
    return 'Warden could not be reached.';
  }
}

export async function refreshUsage(): Promise<void> {
  if (store.refreshing) return;
  store.refreshing = true;
  await act(api.refresh);
  store.now = Date.now();
  store.refreshing = false;
}

/** Accounts shown on the Overview and in the sidebar: switched-off accounts show only in Settings. */
export function usedAccounts(state: State | null): Account[] {
  return state ? [...(state.main ? [state.main] : []), ...state.accounts.filter((a) => a.enabled)] : [];
}
