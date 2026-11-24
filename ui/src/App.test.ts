import { flushPromises, mount } from '@vue/test-utils';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { nextTick } from 'vue';
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
  store.away = false;
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
    const app = mount(App, { attachTo: document.body, global: { stubs: { transition: false } } });
    await flushPromises();
    const main = app.get('main').element;
    // The idle login's tooltip is drawn into <main>, after the page.
    expect(main.childNodes.length).toBeGreaterThan(1);

    const expectSettledPage = (title: string) =>
      vi.waitFor(() => {
        const box = app.get('main > .page-content');
        expect(box.find('.page-enter-active, .page-leave-active').exists()).toBe(false);
        expect(box.element.children).toHaveLength(1);
        expect(app.findAll('main h1').map((heading) => heading.text())).toEqual([title]);
      });

    go('memory');
    await expectSettledPage('Memory');

    go('');
    await expectSettledPage('Overview');
    app.unmount();
  });

  it('enables the page transition by default and swaps immediately when switched off', async () => {
    store.state = state([]);
    store.now = NOW;
    server();
    const app = mount(App, { attachTo: document.body, global: { stubs: { transition: false } } });
    go('memory');
    await nextTick();
    expect(app.get('main .page-enter-active h1').text()).toBe('Memory');
    expect(app.get('main .page-leave-active[inert] h1').text()).toBe('Overview');

    store.state.settings.pageTransitions = false;
    go('');
    await nextTick();
    expect(app.findAll('main h1').map((heading) => heading.text())).toEqual(['Overview']);
    expect(app.find('main .page-enter-active').exists()).toBe(false);
    expect(app.find('main [inert]').exists()).toBe(false);
    app.unmount();
  });

  it('uses the fade-only page transition when reduced motion is requested', async () => {
    vi.stubGlobal('matchMedia', (query: string) => ({
      matches: query === '(prefers-reduced-motion: reduce)',
      addEventListener() {},
      removeEventListener() {},
    }));
    server();
    const app = mount(App, { attachTo: document.body, global: { stubs: { transition: false } } });
    go('memory');
    await nextTick();
    expect(app.get('main .page-fade-enter-active h1').text()).toBe('Memory');
    expect(app.get('main .page-fade-leave-active h1').text()).toBe('Overview');
    expect(app.find('main .page-enter-active').exists()).toBe(false);
    app.unmount();
  });
});
