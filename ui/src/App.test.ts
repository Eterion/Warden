import { flushPromises, mount } from '@vue/test-utils';
import { afterEach, describe, expect, it, vi } from 'vitest';
import App from './App.vue';
import type { MemoryDay } from './api';
import { isoDay } from './memory';
import { go } from './route';
import { store } from './store';
import { NOW, account, state } from './test/state';

/** A day with nothing recorded, for the Memory page and the Overview's Memory card; today's waits stay open. */
function server() {
  vi.stubGlobal('fetch', (path: string, init: RequestInit = {}) => {
    const date = /date=([\d-]+)/.exec(path)?.[1];
    if (date) {
      const day: MemoryDay = {
        date,
        today: date === isoDay(new Date(NOW)),
        days: [],
        series: [],
        minutes: [],
        gaps: [],
        summary: {
          first: null,
          last: null,
          computerTotal: null,
          now: null,
          nowShare: null,
          peak: null,
          peakAt: null,
          peakShare: null,
          series: [],
        },
      };
      return Promise.resolve(new Response(JSON.stringify(day), { status: 200 }));
    }
    return new Promise((_resolve, reject) =>
      init.signal?.addEventListener('abort', () => reject(new DOMException('aborted', 'AbortError'))),
    );
  });
}

afterEach(() => {
  vi.unstubAllGlobals();
  store.state = null;
  go('');
});

describe('App', () => {
  it('switches pages while the page has drawn tooltips into the main landmark', async () => {
    store.now = NOW;
    const usage = {
      readAt: NOW - 60_000,
      fiveHour: { usedPercent: 10, resetsAt: NOW + 3_600_000 },
      weekly: { usedPercent: 20, resetsAt: NOW + 30 * 3_600_000 },
      perModelWeekly: [],
      extra: null,
    };
    // An idle login, whose reading's age explains itself in a tooltip.
    store.state = state([
      account({ id: 'main', label: 'Main', usage }),
      account({ id: 'a1', label: 'Work', usage, usageError: { kind: 'loginExpired' } }),
    ]);
    server();
    go('');
    const app = mount(App, { attachTo: document.body });
    await flushPromises();
    const main = app.get('main').element;
    // The idle login's tooltip is drawn into <main>, after the page.
    expect(main.childNodes.length).toBeGreaterThan(1);

    go('memory');
    await flushPromises();
    expect(app.get('main h1').text()).toBe('Memory');

    go('');
    await flushPromises();
    expect(app.get('main h1').text()).toBe('Overview');
    app.unmount();
  });
});
