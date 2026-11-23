import { flushPromises, mount } from '@vue/test-utils';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { nextTick, ref } from 'vue';
import type { MemoryDay, MemoryMinute, MemorySeries, MemoryWait } from '../api';
import { isoDay } from '../memory';
import { go, route } from '../route';
import { store } from '../store';
import { NOW, account, state } from '../test/state';
import MemoryCard from './MemoryCard.vue';
import OverviewPage from './OverviewPage.vue';

// The chart's box as the window lays it out: 52 px of gutter and a 300 px plot.
vi.mock('@vueuse/core', async (original) => ({
  ...(await original<typeof import('@vueuse/core')>()),
  useElementSize: () => ({ width: ref(352), height: ref(120), stop: () => {} }),
}));

const GIB = 1024 * 1024;
const TODAY = isoDay(new Date(NOW));

function series(n: number, kind: MemorySeries['kind'], accountId: string | null): MemorySeries {
  return {
    series: n,
    kind,
    provider: kind === 'warden' ? null : 'claude',
    accountId,
    account: accountId,
    sessionId: kind === 'warden' ? null : `s${n}`,
    parentSessionId: null,
    name: null,
    project: null,
  };
}

function minute(m: number, s: [number, number][]): MemoryMinute {
  return { m, total: 32 * GIB, used: 16 * GIB, sum: s.reduce((a, [, v]) => a + v, 0), s };
}

/** Today: main and Work from 09:00, Warden closed 10:00 to 12:00, the peak (3 GB) at 09:01. */
function today(extra: Partial<MemoryDay> = {}): MemoryDay {
  const minutes = [
    minute(540, [
      [1, GIB],
      [2, GIB],
      [3, 9 * 1024],
    ]),
    minute(541, [
      [1, 2 * GIB],
      [2, GIB],
      [3, 9 * 1024],
    ]),
    minute(720, [
      [1, GIB],
      [3, 9 * 1024],
    ]),
    minute(781, [[1, GIB]]),
  ];
  return {
    date: TODAY,
    today: true,
    days: [TODAY],
    series: [series(1, 'session', 'main'), series(2, 'session', 'a1'), series(3, 'warden', null)],
    minutes,
    gaps: [{ from: 600, to: 720 }],
    summary: {
      first: 540,
      last: 781,
      computerTotal: 32 * GIB,
      now: null,
      nowShare: null,
      peak: 3 * GIB,
      peakAt: 541,
      peakShare: null,
      series: [],
    },
    ...extra,
  };
}

/** A stand-in server: today's day, the Overview's state, and today's waits from a queue (or held open). */
function server(day: MemoryDay, waits: MemoryWait[] = []) {
  const asked: string[] = [];
  vi.stubGlobal('fetch', (path: string, init: RequestInit = {}) => {
    asked.push(path);
    const ok = (body: unknown) => Promise.resolve(new Response(JSON.stringify(body), { status: 200 }));
    if (path.startsWith('/api/memory/day')) return ok(day);
    if (!path.startsWith('/api/memory/today/wait')) return ok(store.state);
    const next = waits.shift();
    if (next) return ok(next);
    return new Promise((_resolve, reject) =>
      init.signal?.addEventListener('abort', () => reject(new DOMException('aborted', 'AbortError'))),
    );
  });
  return asked;
}

beforeEach(() => {
  store.now = NOW;
  store.away = false;
  store.connected = true;
  store.state = state([account({ id: 'main', label: 'Main' }), account({ id: 'a1', label: 'Work' })]);
  go('');
});

afterEach(() => {
  vi.unstubAllGlobals();
  store.state = null;
  go('');
});

describe('MemoryCard', () => {
  it("shows today's figures beside its heading: the AI sessions now with their share, and the peak with its time", async () => {
    server(today());
    const card = mount(MemoryCard, { attachTo: document.body });
    await flushPromises();
    expect(card.get('h2').text()).toBe('Memory');
    expect(card.get('[data-card-figure="now"]').text()).toBe('1 GB now (3%)');
    expect(card.get('[data-card-figure="peak"]').text()).toBe('3 GB peak at 09:01');
    card.unmount();
  });

  it("draws the page's chart small: the same bands and colours, the gap, two gridlines and the times at its ends", async () => {
    server(today());
    const card = mount(MemoryCard, { attachTo: document.body });
    await flushPromises();
    const chart = card.get('[data-memory-card-chart]');
    expect(chart.findAll('[data-band]').map((b) => [b.attributes('data-band'), b.classes()[0]])).toEqual([
      ['account:main', 'fill-chart-claude-1'],
      ['account:a1', 'fill-chart-claude-2'],
    ]);
    // Shorter than the page's: an 84 px plot, its gridlines at 2 and 4 GB.
    expect(chart.get('svg').attributes('height')).toBe('104');
    expect(chart.findAll('[data-gridline]').map((g) => g.text())).toEqual(['2 GB', '4 GB']);
    // Warden closed from 10:00 to 12:00: an empty band, named.
    expect(chart.findAll('[data-gap]')).toHaveLength(1);
    expect(chart.get('[data-not-running]').text()).toBe('Warden not running');
    // The times only at its ends, inside the plot: the first minute and the latest sample's.
    const labels = chart.findAll('[data-axis-label]');
    expect(labels.map((l) => [l.text(), l.attributes('text-anchor'), l.attributes('x')])).toEqual([
      ['09:00', 'start', '52'],
      ['13:01', 'end', '352'],
    ]);
    // No guide or tooltip here: the minute-by-minute guide stays on the Memory page.
    expect(chart.find('[data-guide]').exists()).toBe(false);
    expect(chart.find('[role=slider]').exists()).toBe(false);
    card.unmount();
  });

  it('leads to the Memory page: the heading and the chart, each one link, the chart named by what it shows', async () => {
    server(today());
    const card = mount(MemoryCard, { attachTo: document.body });
    await flushPromises();
    const links = card.findAll('a');
    expect(links.map((a) => a.attributes('href'))).toEqual(['#/memory', '#/memory']);
    expect(links[1]!.get('svg').attributes('aria-hidden')).toBe('true');
    expect(links[1]!.get('[data-chart-name]').text()).toBe("AI sessions' memory today: now 1 GB, peak 3 GB at 09:01");
    // Nothing in the chart takes focus of its own: the link is the one stop.
    expect(links[1]!.findAll('[tabindex]')).toHaveLength(0);
    location.hash = links[1]!.attributes('href')!;
    window.dispatchEvent(new HashChangeEvent('hashchange'));
    await nextTick();
    expect(route.page.value).toBe('memory');
    expect(route.day.value).toBeNull();
    card.unmount();
  });

  it('says no AI session is running, in place of the figures and the chart, when none ran today', async () => {
    const quiet = today({
      series: [series(3, 'warden', null)],
      minutes: [minute(540, [[3, 9 * 1024]]), minute(541, [[3, 9 * 1024]])],
      gaps: [],
    });
    server(quiet);
    const card = mount(MemoryCard, { attachTo: document.body });
    await flushPromises();
    expect(card.get('[data-card-none]').text()).toBe('No AI sessions are running.');
    expect(card.find('[data-memory-card-chart]').exists()).toBe(false);
    expect(card.find('[data-memory-card-figures]').exists()).toBe(false);
    expect(card.get('h2 a').attributes('href')).toBe('#/memory');
    card.unmount();
  });

  it('follows today through the wait, and stops while the window is away', async () => {
    const later: MemoryWait = { ...today(), minutes: [minute(782, [[1, 4 * GIB]])], sample: 7 };
    later.summary = { ...later.summary, peak: 4 * GIB, peakAt: 782 };
    const asked = server(today(), [later]);
    const card = mount(MemoryCard, { attachTo: document.body });
    await flushPromises();
    await flushPromises();
    const waits = asked.filter((p) => p.includes('/today/wait'));
    expect(waits[0]).toBe('/api/memory/today/wait?after=781');
    expect(waits[1]).toBe('/api/memory/today/wait?after=782&sample=7');
    expect(card.get('[data-card-figure="now"]').text()).toBe('4 GB now (13%)');
    expect(card.get('[data-card-figure="peak"]').text()).toBe('4 GB peak at 13:02');
    expect(card.findAll('[data-axis-label]').map((l) => l.text())).toEqual(['09:00', '13:02']);
    store.away = true;
    await nextTick();
    const count = asked.length;
    await flushPromises();
    expect(asked.length).toBe(count);
    card.unmount();
  });

  it('sits in the Overview above the limit resets, and stops asking once the Overview is left', async () => {
    store.state = state([
      account({
        id: 'main',
        label: 'Main',
        resets: [{ id: 'grant-full', title: 'Full reset', expiresAt: NOW + 2 * 24 * 3_600_000 }],
      }),
    ]);
    const asked = server(today());
    const page = mount(OverviewPage, { attachTo: document.body });
    await flushPromises();
    expect(page.find('[data-memory-card] [data-memory-card-chart]').exists()).toBe(true);
    // One column: the card first, the limit resets under it.
    const column = page.get('[data-memory-card]').element.parentElement!;
    expect([...column.children].map((e) => e.querySelector('h2')?.textContent?.trim())).toEqual([
      'Memory',
      expect.stringMatching(/^Limit resets/),
    ]);
    const count = asked.length;
    expect(asked[count - 1]).toContain('/api/memory/today/wait');
    page.unmount();
    store.away = true;
    await flushPromises();
    store.away = false;
    await flushPromises();
    expect(asked.length).toBe(count);
  });
});
