import { nextTick } from 'vue';
import { describe, expect, it, vi } from 'vitest';
import { state } from './test/state';

// The desktop app's window, with the server stood in for: one long poll that waits until it is aborted.
const signals: AbortSignal[] = [];
vi.mock('./api', async (original) => {
  const real = await original<typeof import('./api')>();
  return {
    ...real,
    api: {
      ...real.api,
      state: () => Promise.resolve(state([])),
      stateAfter: (_: number, signal: AbortSignal) => {
        signals.push(signal);
        return new Promise((_resolve, reject) => signal.addEventListener('abort', () => reject(new Error('aborted'))));
      },
    },
  };
});
vi.mock('./desktop', async () => {
  const { reactive } = await import('vue');
  return { desktop: reactive({ inApp: true, maximized: false, minimized: false }) };
});

const settle = async () => {
  for (let i = 0; i < 5; i++) await nextTick();
  await new Promise((r) => setTimeout(r, 0));
};

describe('store in the desktop app', () => {
  it('stops following the server and pauses spinners while the window is minimized, and resumes when restored', async () => {
    const { desktop } = await import('./desktop');
    const { startStore, store } = await import('./store');
    startStore();
    await settle();
    expect(store.state?.revision).toBe(1);
    expect(signals).toHaveLength(1);
    expect(signals[0]!.aborted).toBe(false);

    desktop.minimized = true;
    await settle();
    expect(signals[0]!.aborted).toBe(true);
    expect(signals).toHaveLength(1);
    expect(document.documentElement.hasAttribute('data-away')).toBe(true);

    desktop.minimized = false;
    await settle();
    expect(signals).toHaveLength(2);
    expect(signals[1]!.aborted).toBe(false);
    expect(document.documentElement.hasAttribute('data-away')).toBe(false);
  });
});
