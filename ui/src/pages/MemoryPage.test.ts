import { flushPromises, mount } from '@vue/test-utils';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { nextTick } from 'vue';
import type { MemoryDay, MemoryMinute, MemorySeries, MemoryWait } from '../api';
import { isoDay } from '../memory';
import { go, route } from '../route';
import { store } from '../store';
import { NOW, account, state } from '../test/state';
import MemoryPage from './MemoryPage.vue';

const GIB = 1024 * 1024;
const TODAY = isoDay(new Date(NOW));
const PAST = '2026-11-05';

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

function minute(m: number, s: [number, number][], used = 16 * GIB): MemoryMinute {
  return { m, total: 32 * GIB, used, sum: s.reduce((a, [, v]) => a + v, 0), s };
}

function day(date: string, minutes: MemoryMinute[], extra: Partial<MemoryDay> = {}): MemoryDay {
  return {
    date,
    today: date === TODAY,
    days: [PAST, TODAY],
    series: [series(1, 'session', 'main'), series(2, 'session', 'a1'), series(3, 'warden', null)],
    minutes,
    gaps: [],
    summary: {
      first: minutes[0]?.m ?? null,
      last: minutes[minutes.length - 1]?.m ?? null,
      computerTotal: minutes.length ? 32 * GIB : null,
      now: null,
      nowShare: null,
      peak: null,
      peakAt: null,
      peakShare: null,
      series: [],
    },
    ...extra,
  };
}

/** A figure's parts: its label, then each line under it, as the page shows them (the meter's line is empty). */
function figure(
  page: { find: (s: string) => { exists: () => boolean; findAll: (s: string) => { text: () => string }[] } },
  key: string,
) {
  const f = page.find(`[data-figure="${key}"]`);
  return f.exists() ? f.findAll('dt, dd').map((e) => e.text().trim()) : null;
}

const busyDay = (date: string) =>
  day(date, [
    minute(540, [
      [1, 2 * GIB],
      [2, GIB],
      [3, 9 * 1024],
    ]),
    minute(541, [
      [1, GIB],
      [2, GIB],
      [3, 9 * 1024],
    ]),
    minute(
      542,
      [
        [1, GIB],
        [3, 9 * 1024],
      ],
      30 * GIB,
    ),
  ]);

/** A stand-in server: the day asked for, and today's waits answered from a queue (or held open). */
function server(days: Record<string, MemoryDay>, waits: MemoryWait[] = []) {
  const asked: string[] = [];
  vi.stubGlobal('fetch', (path: string, init: RequestInit = {}) => {
    asked.push(path);
    const date = /date=([\d-]+)/.exec(path)?.[1];
    if (date) {
      const d = days[date] ?? day(date, []);
      return Promise.resolve(new Response(JSON.stringify(d), { status: 200 }));
    }
    const next = waits.shift();
    if (next) return Promise.resolve(new Response(JSON.stringify(next), { status: 200 }));
    return new Promise((_resolve, reject) =>
      init.signal?.addEventListener('abort', () => reject(new DOMException('aborted', 'AbortError'))),
    );
  });
  return asked;
}

beforeEach(() => {
  store.now = NOW;
  store.away = false;
  store.state = state([account({ id: 'main', label: 'Main' }), account({ id: 'a1', label: 'Work' })]);
  go('memory');
});

afterEach(() => {
  vi.unstubAllGlobals();
  store.state = null;
  go('');
});

describe('the address', () => {
  it('names the Memory page and its day, and keeps the page under a modal', () => {
    go('memory');
    expect(route.page.value).toBe('memory');
    expect(route.day.value).toBeNull();
    go(`memory/${PAST}`);
    expect(route.day.value).toBe(PAST);
    go('settings/general');
    expect(route.page.value).toBe('memory');
    expect(route.day.value).toBe(PAST);
    go('memory/not-a-day');
    expect(route.day.value).toBeNull();
    go('');
    expect(route.page.value).toBe('overview');
  });
});

describe('MemoryPage', () => {
  it('shows today: the figures, the chart and the table, Next and Today saying why they do nothing', async () => {
    server({ [TODAY]: busyDay(TODAY) });
    const page = mount(MemoryPage, { attachTo: document.body });
    await flushPromises();
    expect(page.get('[data-day]').text()).toBe('Monday, 9 November');
    expect(figure(page, 'now')).toEqual(['AI sessions now', '1 GB', '3% of 32 GB']);
    expect(figure(page, 'peak')).toEqual(['Peak today', '3 GB', 'at 09:00']);
    expect(figure(page, 'computer')).toEqual(['Computer', '30 GB', '', '94% of 32 GB']);
    expect(figure(page, 'warden')).toEqual(['Warden itself', '9 MB', '<1% of 32 GB']);
    // Each with its icon, Warden's its own owl.
    expect(page.findAll('[data-figures] dt svg')).toHaveLength(4);
    expect(page.find('[data-figure="warden"] [data-owl-icon]').exists()).toBe(true);
    // The computer above 90% in use: its number, share and meter amber.
    expect(page.get('[data-computer] dd').classes()).toContain('text-attn');
    expect(page.get('[data-computer] dd:last-child').classes()).toContain('text-attn');
    expect(page.get('[data-meter]').attributes('data-high')).toBe('true');
    expect(page.get('[data-meter-used]').classes()).toContain('bg-attn-icon');
    // The meter: Main's 1 GB of the 30 GB in use marked in the text colour, then the rest, as shares of 32 GB.
    expect(page.get('[data-meter-ai]').classes()).toContain('bg-text');
    expect(page.get('[data-meter-ai]').attributes('style')).toContain('width: 3.13%');
    expect(page.get('[data-meter-used]').attributes('style')).toContain('width: 90.63%');
    const rows = page.findAll('[data-memory-table] [role=row]');
    expect(rows.map((r) => r.text())).toEqual([
      'Shown in the chartAccountNowPeakShare now',
      'Main1 GB2 GB3%',
      'Work0 MB1 GB0%',
    ]);
    expect(page.find('[data-band="account:main"]').exists()).toBe(true);
    expect(page.get('button[aria-label="Next day"]').attributes('aria-disabled')).toBe('true');
    expect(page.get('button[aria-disabled]:not([aria-label])').text()).toBe('Today');
    page.unmount();
  });

  it('steps to the nearest recorded day and back to today', async () => {
    server({ [TODAY]: busyDay(TODAY), [PAST]: busyDay(PAST) });
    const page = mount(MemoryPage, { attachTo: document.body });
    await flushPromises();
    await page.get('button[aria-label="Previous day"]').trigger('click');
    await flushPromises();
    expect(location.hash).toBe(`#/memory/${PAST}`);
    expect(page.get('[data-day]').text()).toBe('Thursday, 5 November');
    // A past day: no AI sessions now, its peak, the computer at its highest and when, Warden at its highest.
    expect(figure(page, 'now')).toBeNull();
    expect(figure(page, 'peak')).toEqual(['Peak', '3 GB', 'at 09:00']);
    expect(figure(page, 'computer')).toEqual(['Computer', '30 GB', '', '94% of 32 GB at 09:02']);
    expect(figure(page, 'warden')).toEqual(['Warden itself', '9 MB', 'at its highest']);
    expect(page.get('button[aria-label="Previous day"]').attributes('aria-disabled')).toBe('true');
    await page.get('button[aria-label="Next day"]').trigger('click');
    await flushPromises();
    expect(location.hash).toBe('#/memory');
    page.unmount();
  });

  it("leaves a band out of the chart and the figures with its checkbox, and darkens the hovered row's band", async () => {
    server({ [TODAY]: busyDay(TODAY) });
    const page = mount(MemoryPage, { attachTo: document.body });
    await flushPromises();
    const box = page.get('[role=checkbox][aria-label="Show Work in the chart"]');
    expect(box.attributes('aria-checked')).toBe('true');
    await box.trigger('click');
    expect(page.get('[role=checkbox][aria-label="Show Work in the chart"]').attributes('aria-checked')).toBe('false');
    expect(page.find('[data-band="account:a1"]').exists()).toBe(false);
    expect(figure(page, 'peak')?.[1]).toBe('2 GB');
    // The row stays, with its own figures.
    expect(page.get('[data-band-row="account:a1"]').text()).toContain('1 GB');
    await page.get('[data-band-row="account:main"]').trigger('pointerenter');
    expect(page.get('[data-band="account:main"]').classes()).toContain('fill-chart-hover');
    await page.get('[data-band-row="account:main"]').trigger('pointerleave');
    expect(page.get('[data-band="account:main"]').classes()).toContain('fill-chart-claude-1');
    page.unmount();
  });

  it('colours each band by its account, whatever the checkboxes leave out', async () => {
    server({ [TODAY]: busyDay(TODAY) });
    const page = mount(MemoryPage, { attachTo: document.body });
    await flushPromises();
    const swatch = (key: string) => page.get(`[data-band-row="${key}"] [role=cell] > span[aria-hidden]`).classes();
    expect(page.get('[data-band="account:main"]').classes()).toContain('fill-chart-claude-1');
    expect(page.get('[data-band="account:a1"]').classes()).toContain('fill-chart-claude-2');
    expect(swatch('account:a1')).toContain('bg-chart-claude-2');
    // Main left out: Work keeps its own shade, not the first one.
    await page.get('[role=checkbox][aria-label="Show Main in the chart"]').trigger('click');
    expect(page.find('[data-band="account:main"]').exists()).toBe(false);
    expect(page.get('[data-band="account:a1"]').classes()).toContain('fill-chart-claude-2');
    expect(swatch('account:a1')).toContain('bg-chart-claude-2');
    expect(swatch('account:main')).toContain('bg-chart-claude-1');
    page.unmount();
  });

  it('gives Codex accounts its three slate shades, the fourth the first again, kept when one is left out', async () => {
    const codex = ['x1', 'x2', 'x3', 'x4'];
    store.state = state([
      account({ id: 'main', label: 'Main' }),
      ...codex.map((id) => account({ id, label: id, provider: 'codex' })),
    ]);
    const all = ['main', ...codex];
    server({
      [TODAY]: day(
        TODAY,
        [
          minute(
            540,
            all.map((_, i) => [i + 1, GIB] as [number, number]),
          ),
        ],
        {
          series: all.map((id, i) => ({
            ...series(i + 1, 'session', id),
            provider: id === 'main' ? 'claude' : 'codex',
          })),
        },
      ),
    });
    const page = mount(MemoryPage, { attachTo: document.body });
    await flushPromises();
    const swatch = (key: string) => page.get(`[data-band-row="${key}"] [role=cell] > span[aria-hidden]`).classes();
    const expectShades = (ids: string[]) => {
      for (const id of ids) {
        const shade = { x1: 'codex-1', x2: 'codex-2', x3: 'codex-3', x4: 'codex-1' }[id];
        expect(page.get(`[data-band="account:${id}"]`).classes()).toContain(`fill-chart-${shade}`);
        expect(swatch(`account:${id}`)).toContain(`bg-chart-${shade}`);
      }
    };
    expectShades(codex);
    await page.get('[role=checkbox][aria-label="Show x1 in the chart"]').trigger('click');
    expect(page.find('[data-band="account:x1"]').exists()).toBe(false);
    expectShades(['x2', 'x3', 'x4']);
    page.unmount();
  });

  it('moves the guide with the keyboard, saying what it shows', async () => {
    server({ [TODAY]: busyDay(TODAY) });
    const page = mount(MemoryPage, { attachTo: document.body });
    await flushPromises();
    const chart = page.get('[data-memory-chart]');
    expect(chart.attributes('role')).toBe('slider');
    await chart.trigger('focus');
    expect(chart.attributes('aria-valuetext')).toBe('09:02: Work 0 MB, Main 1 GB, total 1 GB');
    await chart.trigger('keydown', { key: 'Home' });
    expect(chart.attributes('aria-valuetext')).toBe('09:00: Work 1 GB, Main 2 GB, total 3 GB');
    expect(page.get('[data-guide-tip]').text()).toContain('Total3 GB');
    await chart.trigger('keydown', { key: 'ArrowRight' });
    expect(chart.attributes('aria-valuenow')).toBe('541');
    await chart.trigger('keydown', { key: 'End' });
    expect(chart.attributes('aria-valuenow')).toBe('542');
    // Past the last recording (the span is at least an hour), a minute at a time, up to its end.
    await chart.trigger('keydown', { key: 'ArrowRight' });
    expect(chart.attributes('aria-valuetext')).toBe('09:03: Nothing recorded');
    await chart.trigger('keydown', { key: 'PageUp' });
    expect(chart.attributes('aria-valuenow')).toBe('599');
    await chart.trigger('keydown', { key: 'PageDown' });
    expect(chart.attributes('aria-valuenow')).toBe('540');
    // Escape hides the tooltip; the guide stays, and moving it brings the tooltip back.
    await chart.trigger('keydown', { key: 'Escape' });
    expect(page.find('[data-guide-tip]').exists()).toBe(false);
    expect(page.find('[data-guide]').exists()).toBe(true);
    await chart.trigger('keydown', { key: 'ArrowRight' });
    expect(page.find('[data-guide-tip]').exists()).toBe(true);
    await chart.trigger('blur');
    expect(page.find('[data-guide]').exists()).toBe(false);
    page.unmount();
  });

  it('follows today through the wait, and stops while the window is away', async () => {
    const later: MemoryWait = {
      ...busyDay(TODAY),
      minutes: [minute(542, [[1, GIB]]), minute(543, [[1, 4 * GIB]])],
      sample: 7,
    };
    const asked = server({ [TODAY]: busyDay(TODAY) }, [later]);
    const page = mount(MemoryPage, { attachTo: document.body });
    await flushPromises();
    await flushPromises();
    // Asked from the last minute it held, without a sample first; then with the sample the answer named.
    const waits = asked.filter((p) => p.includes('/today/wait'));
    expect(waits[0]).toBe('/api/memory/today/wait?after=542');
    expect(waits[1]).toBe('/api/memory/today/wait?after=543&sample=7');
    expect(figure(page, 'now')?.[1]).toBe('4 GB');
    expect(figure(page, 'peak')).toEqual(['Peak today', '4 GB', 'at 09:03']);
    store.away = true;
    await nextTick();
    const count = asked.length;
    await flushPromises();
    expect(asked.length).toBe(count);
    page.unmount();
  });

  it('steps through a gap a minute at a time, saying Warden was not running', async () => {
    const gappy = busyDay(TODAY);
    gappy.minutes = [gappy.minutes[0]!, { ...gappy.minutes[2]!, m: 545 }];
    gappy.gaps = [{ from: 541, to: 545 }];
    server({ [TODAY]: gappy });
    const page = mount(MemoryPage, { attachTo: document.body });
    await flushPromises();
    const chart = page.get('[data-memory-chart]');
    await chart.trigger('focus');
    await chart.trigger('keydown', { key: 'Home' });
    await chart.trigger('keydown', { key: 'ArrowRight' });
    expect(chart.attributes('aria-valuetext')).toBe('09:01: Warden not running');
    expect(page.get('[data-guide-tip]').text()).toBe('09:01Warden not running');
    for (let i = 0; i < 4; i++) await chart.trigger('keydown', { key: 'ArrowRight' });
    expect(chart.attributes('aria-valuetext')).toMatch(/^09:05: Work 0 MB, Main 1 GB/);
    page.unmount();
  });

  it('stops asking once the page is left, a wait under way included', async () => {
    const asked = server({ [TODAY]: busyDay(TODAY) });
    const page = mount(MemoryPage, { attachTo: document.body });
    await flushPromises();
    const count = asked.length;
    expect(asked[count - 1]).toContain('/today/wait');
    page.unmount();
    store.away = true;
    await flushPromises();
    store.away = false;
    await flushPromises();
    expect(asked.length).toBe(count);
  });

  it('goes to the day picked in the calendar, today to its own address', async () => {
    server({ [TODAY]: busyDay(TODAY), [PAST]: busyDay(PAST) });
    const page = mount(MemoryPage, { attachTo: document.body });
    await flushPromises();
    await page.get('[data-day-picker]').trigger('click');
    await flushPromises();
    // The days with recordings, from the day the page read.
    const marked = [...document.querySelectorAll('[data-calendar] :not([data-outside-view]) > [data-recorded]')];
    expect(marked.map((d) => d.parentElement!.getAttribute('data-value'))).toEqual([PAST, TODAY]);
    document.querySelector<HTMLElement>(`[data-calendar] [data-value="${PAST}"]`)!.click();
    await flushPromises();
    expect(location.hash).toBe(`#/memory/${PAST}`);
    expect(page.get('[data-day]').text()).toBe('Thursday, 5 November');
    await page.get('[data-day-picker]').trigger('click');
    await flushPromises();
    document.querySelector<HTMLElement>(`[data-calendar] [data-value="${TODAY}"]`)!.click();
    await flushPromises();
    expect(location.hash).toBe('#/memory');
    page.unmount();
  });

  it('a past day: the meter marks the AI sessions in the minute the computer was at its highest', async () => {
    server({ [PAST]: busyDay(PAST) });
    go(`memory/${PAST}`);
    const page = mount(MemoryPage, { attachTo: document.body });
    await flushPromises();
    // 09:02, 30 GB in use, Main's 1 GB of it.
    expect(page.get('[data-meter-ai]').attributes('style')).toContain('width: 3.13%');
    // A computer below 90%: no amber.
    page.unmount();
    const calm = busyDay(PAST);
    calm.minutes = calm.minutes.map((m) => ({ ...m, used: 16 * GIB }));
    server({ [PAST]: calm });
    const again = mount(MemoryPage, { attachTo: document.body });
    await flushPromises();
    expect(again.get('[data-meter]').attributes('data-high')).toBeUndefined();
    expect(again.get('[data-meter-used]').classes()).toContain('bg-control');
    expect(again.get('[data-computer] dd').classes()).not.toContain('text-attn');
    again.unmount();
  });

  it('keeps focus on Next as it turns off on arriving at today', async () => {
    server({ [TODAY]: busyDay(TODAY), [PAST]: busyDay(PAST) });
    go(`memory/${PAST}`);
    const page = mount(MemoryPage, { attachTo: document.body });
    await flushPromises();
    const next = page.get('button[aria-label="Next day"]');
    (next.element as HTMLElement).focus();
    await next.trigger('click');
    await flushPromises();
    expect(location.hash).toBe('#/memory');
    const after = page.get('button[aria-label="Next day"]');
    expect(after.attributes('aria-disabled')).toBe('true');
    expect(after.element).toBe(next.element);
    expect(document.activeElement).toBe(next.element);
    page.unmount();
  });

  it('today, once nothing runs any more, says so above the chart of the day', async () => {
    const idle = busyDay(TODAY);
    idle.minutes = [...idle.minutes, minute(543, [[3, 9 * 1024]])];
    server({ [TODAY]: idle });
    const page = mount(MemoryPage, { attachTo: document.body });
    await flushPromises();
    expect(page.get('[data-memory-none-now]').text()).toBe('No AI sessions are running.');
    expect(page.find('[data-memory-chart]').exists()).toBe(true);
    expect(figure(page, 'now')?.[1]).toBe('0 MB');
    page.unmount();
  });

  it('reads a past day once, without waiting for samples', async () => {
    const asked = server({ [PAST]: busyDay(PAST) });
    go(`memory/${PAST}`);
    const page = mount(MemoryPage, { attachTo: document.body });
    await flushPromises();
    expect(asked.filter((p) => p.includes('/today/wait'))).toEqual([]);
    page.unmount();
  });

  it('says when nothing was recorded on a past day', async () => {
    server({});
    go('memory/2026-11-01');
    const page = mount(MemoryPage, { attachTo: document.body });
    await flushPromises();
    expect(page.get('[data-memory-empty]').text()).toBe('Nothing recorded on this day.');
    expect(page.find('[data-memory-table]').exists()).toBe(false);
    page.unmount();
  });

  it('today with no AI sessions running shows the computer and says so in place of the bands', async () => {
    server({ [TODAY]: day(TODAY, [minute(540, [[3, 9 * 1024]], 12 * GIB)]) });
    const page = mount(MemoryPage, { attachTo: document.body });
    await flushPromises();
    expect(page.get('[data-memory-none]').text()).toBe('No AI sessions are running.');
    expect(figure(page, 'computer')).toEqual(['Computer', '12 GB', '', '38% of 32 GB']);
    expect(page.find('[data-memory-chart]').exists()).toBe(false);
    page.unmount();
  });
});
