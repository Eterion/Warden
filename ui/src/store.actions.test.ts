import { flushPromises } from '@vue/test-utils';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { State } from './api';
import { act, apply, refreshUsage, store, usedAccounts } from './store';
import { fakeServer } from './test/api';
import { account, state } from './test/state';

afterEach(() => {
  store.state = null;
  store.connected = true;
  store.refreshing = false;
  delete document.documentElement.dataset.theme;
  vi.unstubAllGlobals();
});

/** A fetch whose answers wait until the test lets each one go, in order. */
function heldServer() {
  const pending: { body: unknown; answer: (s: State) => void }[] = [];
  vi.stubGlobal('fetch', (_path: string, init: RequestInit = {}) => {
    return new Promise<Response>((resolve) => {
      pending.push({
        body: typeof init.body === 'string' ? JSON.parse(init.body) : undefined,
        answer: (s) => resolve(new Response(JSON.stringify(s), { status: 200 })),
      });
    });
  });
  return pending;
}

describe('apply', () => {
  it('takes the new state, marks the server reachable and sets the theme on the page', () => {
    store.connected = false;
    const next = {
      ...state([]),
      settings: {
        reserve: 25,
        usageIntervalMin: 5,
        theme: 'dark' as const,
        keepInTray: false,
        workHours: null,
        resetWarningDays: 3,
        showQuietSessions: true,
        hiddenResetAccounts: [],
        soonestResetOnly: false,
      },
    };
    apply(next);
    expect(store.state).toStrictEqual(next);
    expect(store.connected).toBe(true);
    expect(document.documentElement.dataset.theme).toBe('dark');

    apply(state([]));
    expect(document.documentElement.hasAttribute('data-theme')).toBe(false);
  });
});

describe('act', () => {
  it('applies the answer and returns null when the server accepts', async () => {
    const s = state([account({ id: 'a', label: 'A' })]);
    expect(await act(() => Promise.resolve(s))).toBeNull();
    expect(store.state).toStrictEqual(s);
  });

  it("returns the server's sentence when it refuses, and stays connected", async () => {
    store.state = state([]);
    fakeServer(() => ({ status: 409, error: 'That folder is already an account.' }));
    const { api } = await import('./api');
    expect(await act(() => api.addExisting('claude', 'x', 'X'))).toBe('That folder is already an account.');
    expect(store.connected).toBe(true);
    expect(store.state?.revision).toBe(1);
  });

  it('says Warden could not be reached and marks it so when the request fails', async () => {
    vi.stubGlobal('fetch', () => Promise.reject(new TypeError('Failed to fetch')));
    const { api } = await import('./api');
    expect(await act(api.refresh)).toBe('Warden could not be reached.');
    expect(store.connected).toBe(false);
  });

  it('falls back to the status code when the server answers an error without a sentence', async () => {
    vi.stubGlobal('fetch', () => Promise.resolve(new Response('not json', { status: 500 })));
    const { api } = await import('./api');
    expect(await act(api.refresh)).toBe('Warden answered 500.');
  });
});

describe('refreshUsage', () => {
  it('sends one refresh at a time and clears the busy flag after', async () => {
    store.state = state([]);
    const pending = heldServer();
    const first = refreshUsage();
    const second = refreshUsage();
    expect(store.refreshing).toBe(true);
    await flushPromises();
    expect(pending).toHaveLength(1);
    pending[0]!.answer({ ...state([]), revision: 2 });
    await first;
    await second;
    expect(store.refreshing).toBe(false);
    expect(store.state?.revision).toBe(2);
  });
});

describe('usedAccounts', () => {
  it('leaves out switched-off accounts, and is empty without a state', () => {
    const s = state([
      account({ id: 'a', label: 'A' }),
      account({ id: 'b', label: 'B', enabled: false }),
      account({ id: 'c', label: 'C' }),
    ]);
    expect(usedAccounts(s).map((a) => a.id)).toEqual(['a', 'c']);
    expect(usedAccounts(null)).toEqual([]);
  });
});
