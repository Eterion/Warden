import { flushPromises, mount } from '@vue/test-utils';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { nextTick } from 'vue';
import type { MemoryDay, MemoryMinute, MemorySeries, MemoryWait, SettingsPatch } from '../api';
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

/** The figure's heading, number and detail. */
function figure(
  page: { find: (s: string) => { exists: () => boolean; findAll: (s: string) => { text: () => string }[] } },
  key: string,
) {
  const f = page.find(`[data-figure="${key}"]`);
  return f.exists() ? f.findAll('header > [id], strong, strong + span, [data-share-note]').map((e) => e.text()) : null;
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
      [3, 14 * 1024],
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

const twoMainSessions = () =>
  day(
    TODAY,
    [
      minute(540, [
        [1, 2 * GIB],
        [2, GIB],
      ]),
    ],
    {
      series: [series(1, 'session', 'main'), series(2, 'session', 'main')],
    },
  );

/** A stand-in server: the day asked for, and today's waits answered from a queue (or held open). */
function server(
  days: Record<string, MemoryDay>,
  waits: MemoryWait[] = [],
  saves: SettingsPatch[] = [],
  answerSave?: () => Promise<Response>,
) {
  const asked: string[] = [];
  vi.stubGlobal('fetch', (path: string, init: RequestInit = {}) => {
    asked.push(path);
    if (path === '/api/settings' && typeof init.body === 'string') {
      const patch: SettingsPatch = JSON.parse(init.body);
      saves.push(patch);
      if (answerSave) return answerSave();
      const current = store.state!;
      return Promise.resolve(new Response(JSON.stringify({ ...current, settings: { ...current.settings, ...patch } })));
    }
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
    go('settings/app');
    expect(route.page.value).toBe('memory');
    expect(route.day.value).toBe(PAST);
    go('memory/not-a-day');
    expect(route.day.value).toBeNull();
    // Written right but no real day: today.
    go('memory/2026-02-30');
    expect(route.day.value).toBeNull();
    go('memory/2024-02-29');
    expect(route.day.value).toBe('2024-02-29');
    go('');
    expect(route.page.value).toBe('overview');
  });
});

describe('MemoryPage', () => {
  it('remembers the Computer switch and group checks when Memory is reopened', async () => {
    const saves: SettingsPatch[] = [];
    const withWarden = (date: string) => {
      const recorded = busyDay(date);
      recorded.minutes[0] = minute(540, [
        [1, 2 * GIB],
        [2, GIB],
        [3, GIB / 4],
      ]);
      recorded.minutes[2] = minute(
        542,
        [
          [1, GIB],
          [3, GIB / 2],
        ],
        30 * GIB,
      );
      return recorded;
    };
    server({ [TODAY]: withWarden(TODAY), [PAST]: withWarden(PAST) }, [], saves);
    const page = mount(MemoryPage, { attachTo: document.body });
    await flushPromises();
    const scale = page.get('[role=switch][aria-label="Show on chart"]');
    const warden = page.get('[data-band-row="warden"]');
    expect(scale.attributes('aria-checked')).toBe('false');
    expect(page.get('[data-computer] label').attributes('for')).toBe(scale.attributes('id'));
    expect(page.get('[data-computer] label').text()).toBe('Show on chart');
    expect(page.find('[data-computer-line]').exists()).toBe(false);
    expect(warden.get('[role=checkbox]').attributes('aria-checked')).toBe('false');
    expect(warden.find('[data-owl-icon]').exists()).toBe(true);
    expect(warden.find('[aria-expanded]').exists()).toBe(false);
    expect(page.find('[data-band="warden"]').exists()).toBe(false);
    await scale.trigger('click');
    await page.get('[data-band-row="account:a1"] [role=checkbox]').trigger('click');
    await warden.get('[role=checkbox]').trigger('click');
    await flushPromises();
    expect(saves).toEqual([
      { memoryScale: 'computer' },
      { memoryFilter: { hiddenAccounts: ['a1'], showLeft: true, showWarden: false } },
      { memoryFilter: { hiddenAccounts: ['a1'], showLeft: true, showWarden: true } },
    ]);
    expect(page.find('[data-computer-line]').exists()).toBe(true);
    expect(page.find('[data-band="warden"]').exists()).toBe(true);
    expect(page.get('[data-band-row="account:a1"] [role=checkbox]').attributes('aria-checked')).toBe('false');
    expect(page.get('[data-legend-band="account:a1"]').text()).toBe('Work (hidden)');
    const guide = page.get('[data-memory-chart]');
    await guide.trigger('focus');
    expect(guide.attributes('aria-valuetext')).toContain('Warden itself 512 MB');
    expect(guide.attributes('aria-valuetext')).toContain('Computer in use');
    expect(figure(page, 'now')).toEqual(['AI sessions now', '1.5 GB', '5% of 32 GB']);
    expect(figure(page, 'peak')?.[1]).toBe('2.3 GB');
    expect(page.get('[data-meter-text]').text()).toMatch(/^AI sessions /);
    go('memory/' + PAST);
    await flushPromises();
    expect(page.find('[data-computer-line]').exists()).toBe(true);
    page.unmount();

    const reopened = mount(MemoryPage, { attachTo: document.body });
    await flushPromises();
    expect(reopened.get('[role=switch][aria-label="Show on chart"]').attributes('aria-checked')).toBe('true');
    expect(reopened.get('[data-band-row="warden"] [role=checkbox]').attributes('aria-checked')).toBe('true');
    expect(reopened.get('[data-band-row="account:a1"] [role=checkbox]').attributes('aria-checked')).toBe('false');
    await reopened.get('[role=switch]').trigger('click');
    await flushPromises();
    expect(reopened.find('[data-computer-line]').exists()).toBe(false);
    expect(saves.at(-1)).toEqual({ memoryScale: 'sessions' });
    reopened.unmount();
  });

  it('opens account details by peak, keeps them across days, and highlights and checks individual slices', async () => {
    const detailed = (date: string) =>
      day(
        date,
        [
          minute(540, [
            [1, GIB],
            [2, GIB],
            [4, 2 * GIB],
            [5, 256 * 1024],
            [6, 512 * 1024],
          ]),
        ],
        {
          series: [
            { ...series(1, 'session', 'main'), name: 'Fix login' },
            { ...series(2, 'session', 'a1'), name: 'Review login', parentSessionId: 's1' },
            { ...series(4, 'session', 'main'), name: 'Run the build' },
            { ...series(5, 'left', 'a1'), name: 'Review login', sessionId: 's2', parentSessionId: 's1' },
            { ...series(6, 'other-sign-in', null), name: 'Read the logs', provider: 'codex' },
          ],
        },
      );
    const saves: SettingsPatch[] = [];
    server({ [TODAY]: detailed(TODAY), [PAST]: detailed(PAST) }, [], saves);
    const page = mount(MemoryPage, { attachTo: document.body });
    await flushPromises();
    const main = () => page.get('[data-band-row="account:main"]');
    const mainCheck = () => main().get('[role=checkbox]');
    const first = () => page.get('[data-band-row="account:main:claude:s4"]');
    const slice = () => page.get('[data-slice="account:main:claude:s4"]');
    const disclosure = () => main().get('[aria-expanded]');
    const open = async (group: string) => page.get(`[aria-label="Show the sessions of ${group}"]`).trigger('click');
    const now = () => page.get('[data-figure="now"] strong').text();
    expect(disclosure().attributes('aria-expanded')).toBe('false');
    expect(page.find('[data-session-of]').exists()).toBe(false);
    expect(page.find('[data-slice]').exists()).toBe(false);
    await open('Main');
    expect(disclosure().attributes('aria-expanded')).toBe('true');
    expect(disclosure().attributes('aria-label')).toBe('Hide the sessions of Main');
    expect(mainCheck().attributes('aria-checked')).toBe('true');
    expect(page.findAll('[data-session-of="account:main"]').map((r) => r.get('[data-memory-name]').text())).toEqual([
      'Run the build',
      'Fix login',
    ]);
    expect(page.find('[data-slice-edge]').exists()).toBe(true);
    expect(slice().classes()).toContain('fill-chart-claude-1');
    await first().trigger('pointerover');
    expect(slice().classes()).toContain('fill-chart-hover');
    expect(page.get('[data-memory-highlight]').classes()).toContain('stroke-chart-hover-outline');
    expect(page.get('[data-band="account:main"]').classes()).toContain('fill-chart-claude-1');
    await first().trigger('pointerout');
    await first().get('[role=checkbox]').trigger('focusin');
    expect(slice().classes()).toContain('fill-chart-hover');
    expect(page.get('[data-memory-highlight]').classes()).toContain('stroke-chart-hover-outline');
    await first().get('[role=checkbox]').trigger('focusout');
    await main().trigger('pointerover');
    expect(page.findAll('[data-slice]').every((s) => s.classes().includes('fill-chart-hover'))).toBe(true);
    await main().trigger('pointerout');
    expect(now()).toBe('4.8 GB');
    await first().get('[role=checkbox]').trigger('click');
    expect(mainCheck().attributes('aria-checked')).toBe('mixed');
    expect(page.find('[data-slice="account:main:claude:s4"]').exists()).toBe(false);
    expect(now()).toBe('2.8 GB');
    await disclosure().trigger('click');
    expect(mainCheck().attributes('aria-checked')).toBe('mixed');
    expect(page.find('[data-slice]').exists()).toBe(false);
    await mainCheck().trigger('click');
    expect(now()).toBe('4.8 GB');
    await open('Main');
    await mainCheck().trigger('click');
    await flushPromises();
    expect(mainCheck().attributes('aria-checked')).toBe('false');
    expect(page.find('[data-band="account:main"]').exists()).toBe(false);
    await first().get('[role=checkbox]').trigger('click');
    expect(mainCheck().attributes('aria-checked')).toBe('mixed');
    expect(now()).toBe('3.8 GB');
    await open('Work');
    await open('Left running');
    await open('Other sign-in');
    const child = page.get('[data-session-of="account:a1"]');
    const parent = child.get('[data-started-from]');
    expect(parent.text()).toBe('Fix login');
    expect(child.findAll('[role=cell]')[2]?.element.contains(parent.element)).toBe(true);
    expect(first().find('[data-started-from]').exists()).toBe(false);
    await parent.trigger('click');
    expect(child.get('[role=checkbox]').attributes('aria-checked')).toBe('false');
    expect(page.get('[data-session-of="left"]').text()).toContain('Review login');
    expect(page.find('[data-session-of="other-sign-in"] [aria-label="Codex"]').exists()).toBe(true);
    go(`memory/${PAST}`);
    await flushPromises();
    expect(disclosure().attributes('aria-expanded')).toBe('true');
    expect(mainCheck().attributes('aria-checked')).toBe('mixed');
    expect(page.find('[data-session-of="left"]').exists()).toBe(true);
    await page.get('[data-band-row="other-sign-in"] [role=checkbox]').trigger('click');
    await page.get('[data-band-row="left"] [role=checkbox]').trigger('click');
    await flushPromises();
    page.unmount();
    const reopened = mount(MemoryPage, { attachTo: document.body });
    await flushPromises();
    expect(reopened.find('[data-session-of]').exists()).toBe(false);
    for (const key of ['account:main', 'other-sign-in', 'left']) {
      expect(reopened.get(`[data-band-row="${key}"] [role=checkbox]`).attributes('aria-checked')).toBe('false');
    }
    expect(saves).toEqual([
      { memoryFilter: { hiddenAccounts: ['main'], showLeft: true, showWarden: false } },
      { memoryFilter: { hiddenAccounts: ['main', 'other-sign-in'], showLeft: true, showWarden: false } },
      { memoryFilter: { hiddenAccounts: ['main', 'other-sign-in'], showLeft: false, showWarden: false } },
    ]);
    reopened.unmount();
  });

  it('keeps session choices while a group save is pending and after it is refused', async () => {
    store.state!.settings.memoryFilter.hiddenAccounts = ['main'];
    const saves: SettingsPatch[] = [];
    let answer: (response: Response) => void = () => {};
    const pending = new Promise<Response>((resolve) => {
      answer = resolve;
    });
    server({ [TODAY]: twoMainSessions() }, [], saves, () => pending);
    const page = mount(MemoryPage, { attachTo: document.body });
    await flushPromises();
    await page.get('[aria-label="Show the sessions of Main"]').trigger('click');
    const group = page.get('[data-band-row="account:main"] [role=checkbox]');
    const sessions = page.findAll('[data-session-of="account:main"] [role=checkbox]');
    const checked = () => sessions.map((session) => session.attributes('aria-checked'));
    await sessions[0]!.trigger('click');
    expect(group.attributes('aria-checked')).toBe('mixed');
    expect(checked()).toEqual(['true', 'false']);
    await group.trigger('click');
    await flushPromises();
    expect(saves).toHaveLength(1);
    expect(group.attributes('aria-checked')).toBe('true');
    expect(group.attributes('aria-disabled')).toBe('true');
    expect(checked()).toEqual(['true', 'false']);
    answer(new Response(JSON.stringify({ error: 'That group could not be saved.' }), { status: 400 }));
    await flushPromises();
    expect(group.attributes('aria-checked')).toBe('mixed');
    expect(group.attributes('aria-disabled')).toBeUndefined();
    expect(checked()).toEqual(['true', 'false']);
    expect(store.state!.settings.memoryFilter.hiddenAccounts).toEqual(['main']);
    const error = page.get('p[role=status]');
    expect(error.text()).toBe('That group could not be saved.');
    expect(error.element.nextElementSibling?.querySelector('[data-memory-table]')).toBeTruthy();
    page.unmount();
  });

  it('uses one save to restore every session in a mixed group saved as hidden', async () => {
    store.state!.settings.memoryFilter.hiddenAccounts = ['main'];
    const saves: SettingsPatch[] = [];
    server({ [TODAY]: twoMainSessions() }, [], saves);
    const page = mount(MemoryPage, { attachTo: document.body });
    await flushPromises();
    await page.get('[aria-label="Show the sessions of Main"]').trigger('click');
    const group = page.get('[data-band-row="account:main"] [role=checkbox]');
    const sessions = page.findAll('[data-session-of="account:main"] [role=checkbox]');
    await sessions[0]!.trigger('click');
    expect(group.attributes('aria-checked')).toBe('mixed');
    await group.trigger('click');
    await flushPromises();
    expect(saves).toEqual([{ memoryFilter: { hiddenAccounts: [], showLeft: true, showWarden: false } }]);
    expect(group.attributes('aria-checked')).toBe('true');
    expect(sessions.map((session) => session.attributes('aria-checked'))).toEqual(['true', 'true']);
    page.unmount();
  });

  it('shows today: the figures, the chart and the table, Next and Today saying why they do nothing', async () => {
    server({ [TODAY]: busyDay(TODAY) });
    const page = mount(MemoryPage, { attachTo: document.body });
    await flushPromises();
    expect(page.get('[data-day]').text()).toBe('Monday, 9 November');
    expect(
      page
        .get('header')
        .findAll('button')
        .map((button) => button.attributes('aria-label') ?? button.text()),
    ).toEqual(['Monday, 9 November', 'Previous day', 'Next day', 'Today']);
    expect(page.get('button[aria-label="Previous day"]').classes()).toContain('size-8');
    expect(page.get('button[aria-label="Next day"]').classes()).toContain('size-8');
    expect(page.get('[data-day-picker]').classes()).toEqual(expect.arrayContaining(['h-8', 'text-14', 'text-center']));
    expect(figure(page, 'now')).toEqual(['AI sessions now', '1 GB', '3% of 32 GB']);
    expect(figure(page, 'peak')).toEqual(['Peak today', '3 GB', 'at 09:00']);
    expect(figure(page, 'computer')).toEqual(['Computer', '30 GB', '94% of 32 GB']);
    expect(figure(page, 'warden')).toEqual(['Warden itself', '9 MB', '<1% of 32 GB']);
    // Each with its icon, Warden's its own owl.
    const figures = page.get('[data-figures]');
    expect(figures.findAll('section, h2')).toHaveLength(0);
    for (const widget of figures.findAll('[role=group][data-figure]')) {
      expect(widget.attributes('aria-labelledby')).toBe(widget.get('header > span').attributes('id'));
      expect(widget.get('header + div').find('strong.text-24').exists()).toBe(true);
    }
    const icons = figures.findAll('header svg');
    expect(icons).toHaveLength(4);
    expect(icons.every((icon) => icon.classes().includes('size-4') && icon.classes().includes('text-icon'))).toBe(true);
    expect(page.find('[data-figure="warden"] [data-owl-icon]').exists()).toBe(true);
    expect(page.get('[data-figure="peak"] header + div + .border-t').text()).toBe("9% of the computer's memory");
    const warden = page.get('[data-figure="warden"]');
    const wardenBody = warden.get('header + div');
    const wardenTrend = wardenBody.get('[data-memory-trend].mt-auto');
    expect(wardenTrend.attributes('aria-hidden')).toBe('true');
    expect(wardenTrend.get('.stroke-secondary').attributes('d')).toBe(
      'M0.00,9.14L16.67,9.14L50.00,2.00L83.33,9.14H100.00',
    );
    expect(wardenBody.get('[data-warden-peak-text].sr-only').text()).toBe('Highest 14 MB at 09:01');
    // No footer: the line closes the figure.
    expect(warden.find('header + div + .border-t').exists()).toBe(false);
    expect(warden.text()).not.toContain('Never counted');
    expect(page.get('[data-share-note]').element.tagName).toBe('BUTTON');
    expect(page.get('[data-figure="now"] [data-memory-trend]').attributes('aria-hidden')).toBe('true');
    expect(page.get('[data-figure="now"] [data-memory-trend] .stroke-secondary').attributes('d')).toContain('M');
    // The computer above 90% in use: its number, share and meter amber.
    expect(page.get('[data-computer] strong').classes()).toContain('text-attn');
    expect(page.get('[data-computer] strong + span').classes()).toContain('text-attn');
    expect(page.get('[data-meter]').attributes('data-high')).toBe('true');
    expect(page.get('[data-meter-used]').classes()).toContain('bg-attn-icon');
    // The meter: Main's 1 GB of the 30 GB in use marked in the text colour, then the rest, as shares of 32 GB.
    expect(page.get('[data-meter-ai]').classes()).toContain('bg-text');
    expect(page.get('[data-meter-ai]').attributes('style')).toContain('width: 3.13%');
    expect(page.get('[data-meter-used]').attributes('style')).toContain('width: 90.63%');
    // Said in words for screen readers.
    expect(page.get('[data-meter-text]').text()).toBe('AI sessions 1 GB of it (3%)');
    expect(
      page
        .get('[data-computer-legend]')
        .findAll('span.inline-flex')
        .map((item) => item.text()),
    ).toEqual(['AI sessions', 'Everything else']);
    expect(page.get('[data-computer-legend]').findAll('.size-2\\.5')).toHaveLength(2);
    expect(page.findAll('h2').map((heading) => heading.text())).toContain('Memory through the day');
    expect(page.findAll('h2').map((heading) => heading.text())).not.toContain('Accounts');
    expect(page.get('[data-memory-table]').attributes('aria-label')).toBe('Memory by account');
    const rows = page.findAll('[data-memory-table] [role=row]');
    expect(rows.map((r) => r.text())).toEqual([
      'Shown in the chartAccountStarted fromNowPeakShare now',
      'Main1 GB2 GB3%',
      'Work0 MB1 GB0%',
      'Warden itself9 MB14 MB<1%',
    ]);
    expect(page.find('[data-band="account:main"]').exists()).toBe(true);
    expect(page.get('button[aria-label="Next day"]').attributes('aria-disabled')).toBe('true');
    const today = page.get('button[aria-disabled]:not([aria-label])');
    expect(today.text()).toBe('Today');
    expect(today.classes()).toEqual(expect.arrayContaining(['h-7', 'ml-1']));
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
    expect(figure(page, 'computer')).toEqual(['Computer', '30 GB', '94% of 32 GB at 09:02']);
    expect(figure(page, 'warden')).toEqual(['Warden itself', '14 MB', '<1% of 32 GB at its highest']);
    expect(page.get('[data-warden-peak-text].sr-only').text()).toBe('Highest 14 MB at 09:01');
    expect(page.findAll('[data-memory-trend]')).toHaveLength(1);
    expect(page.get('[data-figure="warden"] [data-memory-trend] .fill-secondary').attributes('d')).toContain('Z');
    expect(page.find('[data-now-meter]').exists()).toBe(false);
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
    const legend = page.get('[aria-label="Chart legend"]');
    const item = legend.get('[data-legend-band="account:a1"]');
    const swatch = item.get('[aria-hidden]');
    expect(legend.findAll('li').map((entry) => entry.text())).toEqual(['Main', 'Work']);
    expect(legend.findAll('button, a, [tabindex]')).toHaveLength(0);
    expect(legend.classes()).toEqual(expect.arrayContaining(['text-12', 'text-secondary']));
    expect(swatch.classes()).toEqual(expect.arrayContaining(['size-2.5', 'bg-chart-claude-2']));
    expect(item.classes()).not.toContain('text-tertiary');
    const trend = () => page.get('[data-figure="now"] [data-memory-trend] .stroke-secondary').attributes('d');
    const wardenTrend = () => page.get('[data-figure="warden"] [data-memory-trend] .stroke-secondary').attributes('d');
    const initialTrend = trend();
    const initialWardenTrend = wardenTrend();
    expect(box.attributes('aria-checked')).toBe('true');
    await box.trigger('click');
    await flushPromises();
    expect(page.get('[role=checkbox][aria-label="Show Work in the chart"]').attributes('aria-checked')).toBe('false');
    expect(page.find('[data-band="account:a1"]').exists()).toBe(false);
    expect(item.classes()).toContain('text-tertiary');
    expect(swatch.classes()).toEqual(expect.arrayContaining(['border', 'border-control']));
    expect(swatch.classes()).not.toContain('bg-chart-claude-2');
    expect(item.text()).toBe('Work (hidden)');
    expect(trend()).not.toBe(initialTrend);
    expect(wardenTrend()).toBe(initialWardenTrend);
    expect(figure(page, 'peak')?.[1]).toBe('2 GB');
    // The row stays, with its own figures.
    expect(page.get('[data-band-row="account:a1"]').text()).toContain('1 GB');
    await page.get('[data-band-row="account:main"]').trigger('pointerover');
    expect(page.get('[data-band="account:main"]').classes()).toContain('fill-chart-hover');
    await page.get('[data-band-row="account:main"]').trigger('pointerout');
    expect(page.get('[data-band="account:main"]').classes()).toContain('fill-chart-claude-1');
    await box.trigger('click');
    await flushPromises();
    expect(item.classes()).not.toContain('text-tertiary');
    expect(swatch.classes()).toContain('bg-chart-claude-2');
    expect(trend()).toBe(initialTrend);
    page.unmount();
  });

  it('scales each Now meter against the largest reading, including when a band is unchecked', async () => {
    const data = busyDay(TODAY);
    data.minutes.push(
      minute(543, [
        [1, 2 * GIB],
        [2, GIB / 4],
      ]),
    );
    server({ [TODAY]: data });
    const page = mount(MemoryPage, { attachTo: document.body });
    await flushPromises();
    const bars = page.findAll('[data-now-meter]');
    expect(bars.map((bar) => bar.attributes('style'))).toEqual(['width: 100%;', 'width: 12.5%;']);
    expect(bars[0]!.classes()).toContain('bg-chart-claude-1');
    expect(bars[1]!.classes()).toContain('bg-chart-claude-2');
    expect(page.find('[data-band-row="warden"] [data-now-meter]').exists()).toBe(false);
    for (const track of page.findAll('[data-now-track]')) {
      expect(track.classes()).toEqual(expect.arrayContaining(['w-8', 'shrink-0', 'bg-track']));
      expect(track.element.nextElementSibling?.classList).toContain('w-14');
      expect(track.element.nextElementSibling?.classList).toContain('text-right');
    }
    for (const bar of bars) {
      expect(bar.classes()).toEqual(expect.arrayContaining(['ring-1', 'ring-panel']));
      expect(bar.element.parentElement?.classList).toContain('inset-px');
    }
    await page.get('[role=checkbox][aria-label="Show Main in the chart"]').trigger('click');
    await flushPromises();
    expect(bars[1]!.attributes('style')).toBe('width: 12.5%;');
    page.unmount();
  });

  it('colours each band by its account, whatever the checkboxes leave out', async () => {
    server({ [TODAY]: busyDay(TODAY) });
    const page = mount(MemoryPage, { attachTo: document.body });
    await flushPromises();
    const swatch = (key: string) => page.get(`[data-band-row="${key}"] [data-memory-swatch]`).classes();
    expect(page.get('[data-band="account:main"]').classes()).toContain('fill-chart-claude-1');
    expect(page.get('[data-band="account:a1"]').classes()).toContain('fill-chart-claude-2');
    expect(swatch('account:a1')).toContain('bg-chart-claude-2');
    // Main left out: Work keeps its own shade, not the first one.
    await page.get('[role=checkbox][aria-label="Show Main in the chart"]').trigger('click');
    await flushPromises();
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
    const swatch = (key: string) => page.get(`[data-band-row="${key}"] [data-memory-swatch]`).classes();
    const expectShades = (ids: string[]) => {
      for (const id of ids) {
        const shade = { x1: 'codex-1', x2: 'codex-2', x3: 'codex-3', x4: 'codex-1' }[id];
        expect(page.get(`[data-band="account:${id}"]`).classes()).toContain(`fill-chart-${shade}`);
        expect(swatch(`account:${id}`)).toContain(`bg-chart-${shade}`);
      }
    };
    expectShades(codex);
    await page.get('[role=checkbox][aria-label="Show x1 in the chart"]').trigger('click');
    await flushPromises();
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
    expect(chart.attributes('aria-valuetext')).toBe('09:02: Main 1 GB, total 1 GB');
    await chart.trigger('keydown', { key: 'Home' });
    expect(chart.attributes('aria-valuetext')).toBe('09:00: Main 2 GB, Work 1 GB, total 3 GB');
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
    for (const trend of page.findAll('[data-memory-trend]')) {
      expect(trend.get('.stroke-secondary').attributes('d')?.match(/M/g)).toHaveLength(2);
      expect(trend.get('.fill-secondary').attributes('d')?.match(/Z/g)).toHaveLength(2);
    }
    await chart.trigger('focus');
    await chart.trigger('keydown', { key: 'Home' });
    await chart.trigger('keydown', { key: 'ArrowRight' });
    expect(chart.attributes('aria-valuetext')).toBe('09:01: Warden not running');
    expect(page.get('[data-guide-tip]').text()).toBe('09:01Warden not running');
    for (let i = 0; i < 4; i++) await chart.trigger('keydown', { key: 'ArrowRight' });
    expect(chart.attributes('aria-valuetext')).toMatch(/^09:05: Main 1 GB/);
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
    expect(again.get('[data-computer] strong').classes()).not.toContain('text-attn');
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
    store.state!.settings.memoryFilter.showWarden = true;
    const idle = busyDay(TODAY);
    idle.minutes = [...idle.minutes, minute(543, [[3, 9 * 1024]])];
    server({ [TODAY]: idle });
    const page = mount(MemoryPage, { attachTo: document.body });
    await flushPromises();
    expect(page.get('[data-memory-none-now]').text()).toBe('No AI sessions are running.');
    expect(page.find('[data-memory-chart]').exists()).toBe(true);
    expect(figure(page, 'now')?.[1]).toBe('9 MB');
    expect(page.findAll('[data-now-track]')).toHaveLength(3);
    expect(page.findAll('[data-now-meter]')).toHaveLength(1);
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
    expect(figure(page, 'computer')).toEqual(['Computer', '12 GB', '38% of 32 GB']);
    expect(page.find('[data-memory-chart]').exists()).toBe(false);
    expect(page.get('[data-band-row="warden"] [role=checkbox]').attributes('aria-checked')).toBe('false');
    page.unmount();
  });
  it('keeps the quiet-day message with Warden included', async () => {
    store.state!.settings.memoryFilter.showWarden = true;
    server({ [TODAY]: day(TODAY, [minute(540, [[3, 9 * 1024]])], { series: [series(3, 'warden', null)] }) });
    const page = mount(MemoryPage, { attachTo: document.body });
    await flushPromises();
    expect(page.get('[data-memory-none-now]').text()).toBe('No AI sessions are running.');
    expect(page.get('[data-band-row="warden"] [role=checkbox]').attributes('aria-checked')).toBe('true');
    expect(page.find('[data-band="warden"]').exists()).toBe(true);
    expect(figure(page, 'now')).toEqual(['AI sessions now', '9 MB', '<1% of 32 GB']);
    page.unmount();
  });

  it('keeps the table available when its checkboxes leave out every recorded band', async () => {
    const todayOnly = day(TODAY, [minute(540, [[1, GIB]])], { series: [series(1, 'session', 'main')] });
    server({ [TODAY]: todayOnly });
    const page = mount(MemoryPage, { attachTo: document.body });
    await flushPromises();
    const main = page.get('[data-band-row="account:main"] [role=checkbox]');
    await main.trigger('click');
    await flushPromises();
    expect(main.attributes('aria-checked')).toBe('false');
    expect(page.get('[data-legend-band="account:main"]').text()).toBe('Main (hidden)');
    expect(page.get('[data-memory-all-hidden]').text()).toBe('Nothing is ticked in the table.');
    await main.trigger('click');
    await flushPromises();
    expect(page.find('[data-memory-chart]').exists()).toBe(true);
    page.unmount();
  });

  it('keeps the Left running row without a band or legend when all its work belongs to hidden accounts', async () => {
    const today = busyDay(TODAY);
    today.series[1] = series(2, 'left', 'a1');
    store.state!.settings.memoryFilter.hiddenAccounts = ['a1'];
    server({ [TODAY]: today });
    const page = mount(MemoryPage, { attachTo: document.body });
    await flushPromises();
    expect(page.get('[data-band-row="left"] [role=checkbox]').attributes('aria-checked')).toBe('true');
    expect(page.find('[data-band="left"], [data-legend-band="left"]').exists()).toBe(false);
    page.unmount();
  });
});
