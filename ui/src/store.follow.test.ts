import { afterEach, describe, expect, it, vi } from 'vitest';
import { ApiError, type State } from './api';
import { state } from './test/state';

// The long poll against a stand-in server whose answers each test scripts: each stateAfter call takes the next one.
type Answer = State | Error | 'hold';
const script: Answer[] = [];
const since: number[] = [];
let loads = 0;

vi.mock('./api', async (original) => {
  const real = await original<typeof import('./api')>();
  return {
    ...real,
    api: {
      ...real.api,
      state: () => {
        loads++;
        return Promise.resolve({ ...state([]), revision: 10 + loads });
      },
      stateAfter: (revision: number, signal: AbortSignal) => {
        since.push(revision);
        const next = script.shift() ?? 'hold';
        if (next === 'hold')
          return new Promise((_r, reject) => signal.addEventListener('abort', () => reject(new Error('aborted'))));
        return next instanceof Error ? Promise.reject(next) : Promise.resolve(next);
      },
    },
  };
});
vi.mock('./desktop', async () => {
  const { reactive } = await import('vue');
  return { desktop: reactive({ inApp: true, maximized: false, minimized: false }) };
});

afterEach(() => {
  vi.useRealTimers();
});

describe('store following the server', () => {
  it('asks again from each answer, marks the server unreachable when it cannot be reached, and retries after 5 s', async () => {
    vi.useFakeTimers();
    script.push(
      { ...state([]), revision: 2 },
      { ...state([]), revision: 3 },
      new TypeError('Failed to fetch'),
      new ApiError(500, 'Something broke.'),
    );
    const { startStore, store } = await import('./store');
    startStore();
    await vi.advanceTimersByTimeAsync(0);

    // First load (revision 11), then the poll asks from 11, 2 and 3 before the failure.
    expect(since.slice(0, 3)).toEqual([11, 2, 3]);
    expect(store.state?.revision).toBe(3);
    expect(store.connected).toBe(false);
    expect(loads).toBe(1);

    // Nothing more until the retry wait is over.
    await vi.advanceTimersByTimeAsync(4_999);
    expect(since).toHaveLength(3);
    await vi.advanceTimersByTimeAsync(1);
    expect(loads).toBe(2);
    expect(store.connected).toBe(true);
    expect(store.state?.revision).toBe(12);
    // The server's own error does not mark it unreachable; after another wait, it loads and polls again.
    expect(since).toEqual([11, 2, 3, 12]);
    expect(store.connected).toBe(true);
    await vi.advanceTimersByTimeAsync(5_000);
    expect(loads).toBe(3);
    expect(since).toEqual([11, 2, 3, 12, 13]);
  });
});
