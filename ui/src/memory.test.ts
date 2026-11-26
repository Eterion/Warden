import { describe, expect, it } from 'vitest';
import type { MemoryDay, MemoryMinute, MemorySeries } from './api';
import {
  CHART,
  CHART_SMALL,
  bandColours,
  bandPaths,
  bandRows,
  bandsOf,
  chartModel,
  computerPath,
  dayTitle,
  endAxis,
  figures,
  guideAt,
  guideText,
  meterParts,
  memoryReadings,
  mergeToday,
  shareText,
  size,
  stepDays,
  timeAxis,
} from './memory';
import {
  DEFAULT_MEMORY_FILTER,
  groupIsShown,
  memoryCatalog,
  memoryGroups,
  sessionBands,
  showsRecordedPeak,
} from './memoryGroups';
import { account } from './test/state';

const GIB = 1024 * 1024;
const MIB = 1024;

function series(
  n: number,
  kind: MemorySeries['kind'],
  accountId: string | null,
  label: string | null = null,
): MemorySeries {
  return {
    series: n,
    kind,
    provider: kind === 'warden' ? null : 'claude',
    accountId,
    account: label,
    sessionId: kind === 'warden' ? null : `s${n}`,
    parentSessionId: null,
    name: null,
    project: null,
  };
}

function minute(m: number, s: [number, number][], used = 16 * GIB): MemoryMinute {
  return { m, total: 32 * GIB, used, sum: s.reduce((a, [, v]) => a + v, 0), s };
}

function day(over: Partial<MemoryDay> & Pick<MemoryDay, 'series' | 'minutes'>): MemoryDay {
  return {
    date: '2026-11-20',
    today: false,
    days: ['2026-11-20'],
    gaps: [],
    summary: {
      first: over.minutes[0]?.m ?? null,
      last: over.minutes[over.minutes.length - 1]?.m ?? null,
      computerTotal: 32 * GIB,
      now: null,
      nowShare: null,
      peak: null,
      peakAt: null,
      peakShare: null,
      series: [],
    },
    ...over,
  };
}

const main = account({ id: 'main', label: 'Main' });
const work = account({ id: 'a1', label: 'Work' });
const off = account({ id: 'a2', label: 'Spare', enabled: false });

describe('account sessions and filtering', () => {
  const history = day({
    today: true,
    series: [
      { ...series(7, 'session', 'main'), name: 'Check the build' },
      { ...series(2, 'session', 'a1'), name: 'Review the API', parentSessionId: 's1' },
      { ...series(1, 'session', 'main'), name: 'Fix the login page' },
      { ...series(3, 'left', 'a1'), sessionId: 's2', name: 'Review the API', parentSessionId: 's1' },
      { ...series(4, 'other-sign-in', null), provider: 'codex', parentSessionId: 'missing' },
      series(5, 'warden', null),
      { ...series(6, 'left', 'main'), name: 'Long-running build' },
    ],
    minutes: [
      minute(600, [
        [1, GIB],
        [2, GIB],
        [3, 100 * MIB],
        [4, 200 * MIB],
        [5, 25 * MIB],
        [6, 50 * MIB],
        [7, 300 * MIB],
      ]),
    ],
  });
  const catalog = memoryCatalog(history.series, [main, work], [main, work]);
  const groups = (filter = DEFAULT_MEMORY_FILTER) => memoryGroups(catalog, filter.hiddenAccounts);
  const sessions = (b: ReturnType<typeof groups>[number]) =>
    bandRows(history, sessionBands(catalog, b, history.date)).sort((a, b) => b.peak - a.peak);

  it('lists sessions by peak within their groups, with provider marks and recorded delegation', () => {
    const bands = groups();
    expect(
      bands.map((b) => [b.label, sessions(b).map(({ band }) => [band.label, band.colour, band.provider, band.from])]),
    ).toEqual([
      [
        'Main',
        [
          ['Fix the login page', 'claude-1', 'claude', null],
          ['Check the build', 'claude-1', 'claude', null],
        ],
      ],
      ['Work', [['Review the API', 'claude-2', 'claude', 'Fix the login page']]],
      ['Other sign-in', [['Session s4', 'other-sign-in', 'codex', null]]],
      [
        'Left running',
        [
          ['Review the API', 'left', 'claude', 'Fix the login page'],
          ['Long-running build', 'left', 'claude', null],
        ],
      ],
      ['Warden itself', []],
    ]);
    const filtered = groups({ ...DEFAULT_MEMORY_FILTER, hiddenAccounts: ['main'] });
    expect(sessions(filtered.find((band) => band.key === 'account:a1')!)[0]?.band.from).toBe('Fix the login page');
  });

  it('keeps Warden as an opt-in row even without a recorded series', () => {
    const empty = memoryCatalog([], [main, work], [main, work]);
    const bands = memoryGroups(empty, []);
    const warden = bands.at(-1)!;
    expect(warden.kind).toBe('warden');
    expect(groupIsShown(warden, DEFAULT_MEMORY_FILTER)).toBe(false);
    expect(groupIsShown(warden, { ...DEFAULT_MEMORY_FILTER, showWarden: true })).toBe(true);
    expect(memoryReadings({ minutes: [], series: [] }, bands).warden).toBeNull();
  });

  it('shares complete bands with the stack and keeps a partial band and its open rows separate', () => {
    const bands = groups();
    const main = bands[0]!;
    const recorded = sessionBands(catalog, main, history.date);
    const kept = recorded.find((slice) => slice.series[0] === 7)!;
    const visible = [{ ...main, series: kept.series, slices: [kept] }, ...bands.slice(1)];
    const values = memoryReadings(history, bands, visible, new Map([[main.key, recorded]]));
    expect(values.rows.get(main.key)?.values[0]).toBe(GIB + 300 * MIB);
    expect(values.shown.get(main.key)?.values[0]).toBe(300 * MIB);
    expect(values.rows.get(kept.key)?.values[0]).toBe(300 * MIB);
    expect(values.rows.get('account:a1')).toBe(values.shown.get('account:a1'));
    expect(values.rows.size).toBe(bands.length + recorded.length);
  });

  it('draws an open group over the same band with internal edges and its own colour', () => {
    const bands = groups();
    const mainBand = bands[0]!;
    const shown = new Set(bands.map((b) => b.key));
    const closed = bandPaths(chartModel(history, bands, shown)!, 60);
    const open = bandPaths(
      chartModel(history, [{ ...mainBand, slices: sessions(mainBand).map((s) => s.band) }, ...bands.slice(1)], shown)!,
      60,
    );
    expect(open.filter((path) => !path.group)).toEqual(closed);
    const slices = open.filter((path) => path.group === mainBand.key);
    expect(slices.map((path) => path.colour)).toEqual(['claude-1', 'claude-1']);
    expect(slices[0]?.edge).toBeTruthy();
    expect(slices[1]?.edge).toBe('');
  });

  it('keeps unchecked groups in the table, and excludes their work from the chart and figures', () => {
    const filter = { hiddenAccounts: ['a1'], showLeft: true, showWarden: true };
    const bands = groups(filter);
    expect(bands.map((b) => b.key)).toEqual(['account:main', 'account:a1', 'other-sign-in', 'left', 'warden']);
    expect(bands.find((b) => b.kind === 'left')?.series).toEqual([6]);
    const visible = bands.filter((b) => groupIsShown(b, filter));
    expect(visible.map((b) => b.key)).toEqual(['account:main', 'other-sign-in', 'left', 'warden']);
    expect(visible.flatMap((b) => b.series).sort()).toEqual([1, 4, 5, 6, 7]);
    const shown = new Set(visible.map((b) => b.key));
    const stats = figures(history, bands, shown, showsRecordedPeak(history, visible));
    expect(stats.now).toBe(GIB + 575 * MIB);
    expect(stats.peak).toBe(stats.now);
    shown.delete('warden');
    expect(figures(history, bands, shown, false).now).toBe(GIB + 550 * MIB);
    const greysOff = { ...DEFAULT_MEMORY_FILTER, hiddenAccounts: ['other-sign-in'], showLeft: false };
    expect(
      groups(greysOff)
        .filter((b) => groupIsShown(b, greysOff))
        .map((b) => b.key),
    ).toEqual(['account:main', 'account:a1']);
  });
});

describe('whole computer and guide', () => {
  it('uses the physical total for the axis and breaks the used line at missing minutes', () => {
    const history = day({
      series: [],
      minutes: [minute(600, [], 8 * GIB), minute(601, [], 16 * GIB), minute(603, [], 24 * GIB)],
      gaps: [{ from: 602, to: 603 }],
    });
    const model = chartModel(history, [], new Set(), 3, 4, 'computer')!;
    expect(model.top).toBe(32 * GIB);
    expect(model.grid.map((line) => line.label)).toEqual(['10 GB', '20 GB', '30 GB']);
    const smaller = { ...history, minutes: history.minutes.map((m) => ({ ...m, total: 16 * GIB })) };
    expect(chartModel(smaller, [], new Set(), 3, 4, 'computer')?.grid.map((line) => line.label)).toEqual([
      '5 GB',
      '10 GB',
      '15 GB',
    ]);
    expect(computerPath(model, 80, CHART)).toBe('M0,152L10,152L30,104H40M60,56L70,56H80');
    expect(guideAt(model, 601).computer).toBe(16 * GIB);
    expect(guideText(guideAt(model, 602))).toBe('10:02: Warden not running');
    expect(computerPath(chartModel(history, [], new Set())!, 80, CHART)).toBe('');
  });

  it('lists groups with memory at the minute and caps sessions of each open group at five', () => {
    const seriesList = [
      ...Array.from({ length: 12 }, (_, i) => series(i + 1, 'session', 'main')),
      ...Array.from({ length: 8 }, (_, i) => series(i + 13, 'session', 'a1')),
      ...Array.from({ length: 9 }, (_, i) => series(i + 21, 'session', `a${i + 2}`)),
    ];
    const history = day({
      series: seriesList,
      minutes: [
        minute(
          600,
          seriesList.slice(0, 20).map((s) => [s.series, (s.series > 12 ? s.series - 12 : s.series) * MIB]),
        ),
      ],
    });
    const catalog = memoryCatalog(history.series, [main, work], [main, work]);
    const bands = catalog.bands.filter((b) => groupIsShown(b, DEFAULT_MEMORY_FILTER));
    const open = bands.map((group) => ({ ...group, slices: sessionBands(catalog, group, history.date) }));
    const model = chartModel(history, open, new Set(open.map((b) => b.key)), 3, 60, 'computer')!;
    const guide = guideAt(model, 600);
    expect(guide.rows).toHaveLength(2);
    expect(guide.rows.slice(0, 2).map(({ kib }) => kib / MIB)).toEqual([78, 36]);
    expect(guide.rows[0]?.sessions?.map(({ kib }) => kib / MIB)).toEqual([12, 11, 10, 9, 8]);
    expect(guide.rows[0]?.more).toEqual({ count: 7, kib: 28 * MIB });
    expect(guide.rows[1]?.sessions?.map(({ kib }) => kib / MIB)).toEqual([8, 7, 6, 5, 4]);
    expect(guide.rows[1]?.more).toEqual({ count: 3, kib: 6 * MIB });
    expect(guide.total).toBe(114 * MIB);
    expect(guideText(guide)).toContain('Session s12 under Main 12 MB');
    expect(guideText(guide)).toContain('7 more, 28 MB included in Main');
    expect(guideText(guide)).toContain('total 114 MB, Computer in use 16 GB');
  });
});

describe('days', () => {
  it('names a day in English, with the year only when it is not this one', () => {
    expect(dayTitle('2026-11-23', '2026-11-23')).toBe('Monday, 23 November');
    expect(dayTitle('2025-12-31', '2026-01-02')).toBe('Wednesday, 31 December 2025');
  });

  it('steps to the nearest recorded day, Next stopping at today', () => {
    const days = ['2026-11-10', '2026-11-18', '2026-11-20'];
    expect(stepDays(days, '2026-11-23', '2026-11-23')).toEqual({ prev: '2026-11-20', next: null });
    expect(stepDays(days, '2026-11-18', '2026-11-23')).toEqual({ prev: '2026-11-10', next: '2026-11-20' });
    // The last recorded day before today goes on to today; the first has nothing before it.
    expect(stepDays(days, '2026-11-20', '2026-11-23')).toEqual({ prev: '2026-11-18', next: '2026-11-23' });
    expect(stepDays(days, '2026-11-10', '2026-11-23')).toEqual({ prev: null, next: '2026-11-18' });
    // A day with nothing (reached by its address) steps around it too; today itself counts as recorded.
    expect(stepDays([...days, '2026-11-23'], '2026-11-15', '2026-11-23')).toEqual({
      prev: '2026-11-10',
      next: '2026-11-18',
    });
  });
});

describe('sizes', () => {
  it('writes GB from 1 GB with one decimal, whole MB below it, in English', () => {
    expect(size(3.44 * GIB)).toBe('3.4 GB');
    expect(size(32 * GIB)).toBe('32 GB');
    expect(size(1234 * GIB)).toBe('1,234 GB');
    expect(size(812 * MIB)).toBe('812 MB');
    expect(size(0)).toBe('0 MB');
  });

  it('writes a share of the computer', () => {
    expect(shareText(3.5 * GIB, 32 * GIB)).toBe('11%');
    expect(shareText(10 * MIB, 32 * GIB)).toBe('<1%');
    expect(shareText(0, 32 * GIB)).toBe('0%');
    expect(shareText(1, null)).toBe('');
  });
});

describe('bands', () => {
  const d = day({
    series: [
      series(1, 'session', 'a1', 'Work'),
      series(2, 'session', 'main', 'Main'),
      series(3, 'left', 'a1', 'Work'),
      series(4, 'warden', null),
      series(5, 'other-sign-in', null),
      series(6, 'session', 'a2', 'Spare'),
      series(7, 'session', 'gone', 'Old account'),
      series(8, 'session', 'main', 'Main'),
    ],
    minutes: [
      minute(600, [
        [1, GIB],
        [2, 2 * GIB],
        [3, 100 * MIB],
        [4, 9 * MIB],
        [8, GIB],
      ]),
    ],
  });

  it('makes a band per account, the used ones always, then Other sign-in and Left running, never Warden', () => {
    const bands = bandsOf(d, [main, work], [main, work, off]);
    expect(bands.map((b) => b.label)).toEqual([
      'Main',
      'Work',
      'Spare',
      'Old account',
      'Other sign-in',
      'Left running',
    ]);
    expect(bands[0]!.series).toEqual([2, 8]);
    expect(bands.find((b) => b.key === 'left')!.series).toEqual([3]);
    // Claude's shades in settings order (Spare, switched off, keeps its place), the account no longer in Settings a
    // fourth that starts them again, and the groups' greys.
    expect(bands.map((b) => b.colour)).toEqual([
      'claude-1',
      'claude-2',
      'claude-3',
      'claude-1',
      'other-sign-in',
      'left',
    ]);
    expect(bands[3]!.account).toEqual({ id: 'gone', provider: 'claude', label: 'Old account' });
  });

  it('colours an account by its place in Settings among its provider, never by what the day shows', () => {
    const codex = account({ id: 'c1', label: 'Codex', provider: 'codex' });
    const quiet = day({ series: [series(1, 'session', 'main')], minutes: [minute(600, [[1, GIB]])] });
    // Work takes Claude's third shade, after Spare in Settings though Spare does not show; Codex its own first.
    const settings = [main, off, work, codex];
    const bands = bandsOf(quiet, [main, work, codex], settings);
    expect(bands.map((b) => [b.label, b.colour])).toEqual([
      ['Main', 'claude-1'],
      ['Work', 'claude-3'],
      ['Codex', 'codex-1'],
    ]);
  });

  it('gives each provider three shades, then starts again, and keeps an account its colour', () => {
    const a = (id: string, provider: 'claude' | 'codex') => ({ id, provider });
    const colours = bandColours(
      [a('m', 'claude'), a('x1', 'codex'), a('c2', 'claude'), a('x2', 'codex'), a('x3', 'codex'), a('x4', 'codex')],
      [a('gone', 'claude'), a('c2', 'claude')],
    );
    expect(Object.fromEntries(colours)).toEqual({
      m: 'claude-1',
      x1: 'codex-1',
      c2: 'claude-2',
      x2: 'codex-2',
      x3: 'codex-3',
      x4: 'codex-1',
      gone: 'claude-3',
    });
  });

  it('shows a switched-off account only on a day it ran something', () => {
    const quiet = day({ series: [series(1, 'session', 'main')], minutes: [minute(600, [[1, GIB]])] });
    expect(bandsOf(quiet, [main, work], [main, work, off]).map((b) => b.label)).toEqual(['Main', 'Work']);
  });
});

describe('figures', () => {
  const d = day({
    today: true,
    series: [series(1, 'session', 'main'), series(2, 'session', 'a1'), series(3, 'warden', null)],
    minutes: [
      minute(
        600,
        [
          [1, 2 * GIB],
          [2, GIB],
          [3, 8 * MIB],
        ],
        20 * GIB,
      ),
      minute(
        601,
        [
          [1, GIB],
          [2, 3 * GIB],
          [3, 9 * MIB],
        ],
        30 * GIB,
      ),
    ],
  });
  const bands = bandsOf(d, [main, work], [main, work]);

  it('adds up the bands shown: now and its share, the peak and its minute, each row', () => {
    // Every band: the peak Warden recorded at once (the server's), not the bands' highs added up.
    const recorded = { ...d, summary: { ...d.summary, peak: 3.5 * GIB, peakAt: 600 } };
    expect(figures(recorded, bands, new Set(['account:main', 'account:a1'])).peak).toBe(3.5 * GIB);
    expect(figures(recorded, bands, new Set(['account:main'])).peak).toBe(2 * GIB);
    const f = figures(d, bands, new Set(['account:main', 'account:a1']));
    expect(f.now).toBe(4 * GIB);
    expect(f.nowShare).toBe('13%');
    expect(f.peak).toBe(4 * GIB);
    expect(f.peakAt).toBe(601);
    expect(f.warden).toBe(9 * MIB);
    expect(f.rows.map((r) => [r.now, r.peak])).toEqual([
      [GIB, 2 * GIB],
      [3 * GIB, 3 * GIB],
    ]);
    // Leaving Work out: only Main counts.
    const mainOnly = figures(d, bands, new Set(['account:main']));
    expect(mainOnly.now).toBe(GIB);
    expect(mainOnly.peak).toBe(2 * GIB);
    expect(mainOnly.peakAt).toBe(600);
  });

  it("marks the computer above 90% in use, with the shown bands' part of it in that minute", () => {
    expect(figures(d, bands, new Set()).computer).toEqual({
      used: 30 * GIB,
      total: 32 * GIB,
      high: true,
      ai: 0,
      at: 601,
    });
    expect(figures(d, bands, new Set(['account:main', 'account:a1'])).computer?.ai).toBe(4 * GIB);
    expect(figures(d, bands, new Set(['account:a1'])).computer?.ai).toBe(3 * GIB);
    const calm = { ...d, minutes: [minute(600, [[1, GIB]], 28 * GIB)] };
    expect(figures(calm, bands, new Set()).computer?.high).toBe(false);
  });

  it('on a past day, gives no now, and the computer and Warden at their highest', () => {
    const past = { ...d, today: false };
    const f = figures(past, bands, new Set(['account:main']));
    expect(f.now).toBeNull();
    expect(f.rows[0]!.now).toBeNull();
    expect(f.computer?.used).toBe(30 * GIB);
    expect(f.computer?.at).toBe(601);
    expect(f.computer?.ai).toBe(GIB);
    expect(f.warden).toBe(9 * MIB);
  });

  it("splits the computer's meter into the AI sessions' part and the rest of the memory in use", () => {
    expect(meterParts({ used: 16 * GIB, total: 32 * GIB, ai: 4 * GIB })).toEqual({ ai: 12.5, rest: 37.5 });
    expect(meterParts({ used: 30 * GIB, total: 32 * GIB, ai: 0 })).toEqual({ ai: 0, rest: 93.75 });
    // Never past the memory in use, nor the computer's total.
    expect(meterParts({ used: 2 * GIB, total: 32 * GIB, ai: 3 * GIB })).toEqual({ ai: 6.25, rest: 0 });
    expect(meterParts({ used: 40 * GIB, total: 32 * GIB, ai: 0 })).toEqual({ ai: 0, rest: 100 });
  });
});

describe('the chart', () => {
  const d = day({
    series: [series(1, 'session', 'main'), series(2, 'session', 'a1')],
    minutes: [
      minute(600, [
        [1, GIB],
        [2, GIB],
      ]),
      minute(601, [[1, GIB]]),
      minute(700, [[2, 2 * GIB]]),
    ],
    gaps: [{ from: 602, to: 700 }],
  });
  const bands = bandsOf(d, [main, work], [main, work]);
  const both = new Set(['account:main', 'account:a1']);

  it('spans the recorded minutes, at least an hour, with three gridlines in GB above the peak', () => {
    const model = chartModel(d, bands, both)!;
    expect([model.from, model.to]).toEqual([600, 701]);
    expect(model.top).toBe(3 * GIB);
    expect(model.grid.map((g) => g.label)).toEqual(['1 GB', '2 GB', '3 GB']);
    const short = chartModel({ ...d, minutes: [minute(1430, [[1, 100 * MIB]])], gaps: [] }, bands, both)!;
    expect([short.from, short.to]).toEqual([1380, 1440]);
    expect(short.grid.map((g) => g.label)).toEqual(['0.1 GB', '0.2 GB', '0.3 GB']);
    expect(chartModel({ ...d, minutes: [] }, bands, both)).toBeNull();
  });

  it('stacks the bands minute by minute, breaking each where nothing was recorded', () => {
    const model = chartModel(d, bands, both)!;
    // 101 minutes over 101 px: a pixel a minute; the plot runs from y 8 (3 GB) to 200 (0).
    const [mainPath, workPath] = bandPaths(model, 101);
    // Two runs each (600–601 and 700): two areas.
    expect(mainPath!.area.match(/M/g)).toHaveLength(2);
    expect(workPath!.area.match(/M/g)).toHaveLength(2);
    // Main sits on the baseline at 1 GB (y 136); Work on top of it in minute 600 (2 GB, y 72), alone at 700 (2 GB).
    expect(mainPath!.area.startsWith('M0,136H2L2,200H0Z')).toBe(true);
    expect(workPath!.area).toContain('M0,72H1V136H2');
    expect(workPath!.area).toContain('M100,72H101L101,200H100Z');
    // Left out, a band is not drawn and the others close up.
    const alone = bandPaths(chartModel(d, bands, new Set(['account:a1']))!, 101);
    expect(alone.map((p) => p.key)).toEqual(['account:a1']);
  });

  it("draws the Overview's small chart on the same model: two gridlines, its own height, the times at its ends", () => {
    const small = chartModel(d, bands, both, 2)!;
    expect([small.from, small.to]).toEqual([600, 701]);
    expect(small.top).toBe(2 * GIB);
    expect(small.grid.map((g) => g.label)).toEqual(['1 GB', '2 GB']);
    // The same bands, on the small chart's plot: from y 6 (2 GB) to 84 (0).
    const [mainPath] = bandPaths(small, 101, CHART_SMALL);
    expect(mainPath!.area.startsWith('M0,45H2L2,84')).toBe(true);
    // Only the two ends, labelled inside the plot: the first minute and the last recorded one.
    const axis = endAxis(small, 300);
    expect(axis.labels).toEqual([
      { m: 600, x: 0, label: '10:00', anchor: 'start' },
      { m: 700, x: 300, label: '11:40', anchor: 'end' },
    ]);
    expect(axis.ticks.map((t) => t.x)).toEqual([0, 300]);
    // Ten minutes recorded: the small chart spans only them (the page's widens to an hour), its labels the real times.
    const tenMinutes = { ...d, minutes: [minute(540, [[1, GIB]]), minute(549, [[1, GIB]])], gaps: [] };
    expect(chartModel(tenMinutes, bands, both)!.to).toBe(600);
    const short = chartModel(tenMinutes, bands, both, 2, 1)!;
    expect([short.from, short.to]).toEqual([540, 550]);
    expect(endAxis(short, 300).labels.map((l) => l.label)).toEqual(['09:00', '09:09']);
    // One minute recorded: one label.
    const one = chartModel({ ...d, minutes: [minute(540, [[1, GIB]])], gaps: [] }, bands, both, 2, 1)!;
    expect([one.from, one.to]).toEqual([540, 541]);
    expect(endAxis(one, 300).labels.map((l) => l.label)).toEqual(['09:00']);
  });

  it('labels the axis at a step giving four to eight labels, with ticks at half the step', () => {
    const span = (from: number, to: number) =>
      chartModel({ ...d, minutes: [minute(from, []), minute(to - 1, [])], gaps: [] }, bands, both)!;
    const labels = (a: ReturnType<typeof timeAxis>) => a.labels.map((l) => l.label);
    // 08:00 to 18:00 (its last minute 17:59): two hours.
    const day10 = timeAxis(span(480, 1080), 800);
    expect(day10.step).toBe(120);
    expect(labels(day10)).toEqual(['08:00', '10:00', '12:00', '14:00', '16:00']);
    expect(day10.ticks.map((t) => t.m)).toEqual([480, 540, 600, 660, 720, 780, 840, 900, 960, 1020, 1080]);
    expect(day10.ticks.filter((t) => t.major).map((t) => t.m)).toEqual([480, 600, 720, 840, 960, 1080]);
    // A whole day: three hours.
    expect(timeAxis(span(0, 1440), 900).step).toBe(180);
    expect(labels(timeAxis(span(0, 1440), 900))).toEqual([
      '00:00',
      '03:00',
      '06:00',
      '09:00',
      '12:00',
      '15:00',
      '18:00',
      '21:00',
    ]);
    // Seven hours: an hour.
    expect(timeAxis(span(480, 900), 900).step).toBe(60);
    // Three hours: half an hour.
    expect(timeAxis(span(480, 660), 900).step).toBe(30);
  });

  it('gives a short span early in the day enough labels, none cut at either end or crowded', () => {
    // Recorded 00:05 to 00:50: the chart spans an hour from its first recording, labelled every quarter.
    const model = chartModel({ ...d, minutes: [minute(5, []), minute(50, [])], gaps: [] }, bands, both)!;
    expect([model.from, model.to]).toEqual([5, 65]);
    const axis = timeAxis(model, 600);
    expect(axis.step).toBe(15);
    expect(axis.labels.map((l) => l.label)).toEqual(['00:15', '00:30', '00:45', '01:00']);
    expect(axis.ticks.map((t) => t.m)).toEqual([7.5, 15, 22.5, 30, 37.5, 45, 52.5, 60]);
    for (const l of axis.labels) expect(l.x + 18).toBeLessThanOrEqual(600);
    // An hour starting just after a whole one: its last label, at the very end, moves inward rather than going.
    for (const [from, width, first] of [
      [1, 600, '00:15'],
      [481, 934, '08:15'],
    ] as const) {
      const hour = chartModel({ ...d, minutes: [minute(from, []), minute(from + 59, [])], gaps: [] }, bands, both)!;
      const a = timeAxis(hour, width);
      expect(a.labels.map((l) => l.label)).toHaveLength(4);
      expect(a.labels[0]!.label).toBe(first);
      const lastLabel = a.labels[a.labels.length - 1]!;
      expect(lastLabel.x).toBe(width - 18);
      // Its tick stays at its time.
      expect(a.ticks.find((t) => t.m === lastLabel.m)!.x).toBeGreaterThan(lastLabel.x);
    }
    // Narrower, the step grows so labels keep their room.
    const narrow = timeAxis(model, 300);
    expect(narrow.step).toBe(15);
    const roomy = timeAxis(model, 150);
    expect(roomy.step).toBe(30);
    for (const axis2 of [narrow, roomy, timeAxis(model, 60)]) {
      const xs = axis2.labels.map((l) => l.x);
      for (let i = 1; i < xs.length; i++) expect(xs[i]! - xs[i - 1]!).toBeGreaterThanOrEqual(40);
    }
    // Too narrow even for three hours over a whole day: every other label, so none collide.
    const wholeDay = chartModel({ ...d, minutes: [minute(0, []), minute(1439, [])], gaps: [] }, bands, both)!;
    const tight = timeAxis(wholeDay, 260);
    const xs = tight.labels.map((l) => l.x);
    for (let i = 1; i < xs.length; i++) expect(xs[i]! - xs[i - 1]!).toBeGreaterThanOrEqual(40);
    expect(tight.labels.length).toBeGreaterThanOrEqual(3);
  });

  it('says what the guide shows: the largest bands with memory that minute first, and their total', () => {
    const model = chartModel(d, bands, both)!;
    const g = guideAt(model, 601);
    expect(g).toEqual({
      time: '10:01',
      rows: [{ key: 'account:main', label: 'Main', kib: GIB }],
      total: GIB,
      missing: null,
    });
    expect(guideText(g)).toBe('10:01: Main 1 GB, total 1 GB');
    // In a gap, and after the last recording of a span stretched to an hour.
    expect(guideText(guideAt(model, 650))).toBe('10:50: Warden not running');
    const short = chartModel({ ...d, minutes: [minute(600, [[1, GIB]])], gaps: [] }, bands, both)!;
    expect(guideText(guideAt(short, 630))).toBe('10:30: Nothing recorded');
  });
});

describe('following today', () => {
  it('replaces the minutes from the first one a later answer brings', () => {
    const held = day({ today: true, series: [], minutes: [minute(600, []), minute(601, [[1, 5]])] });
    const later = day({
      today: true,
      series: [series(1, 'session', 'main')],
      minutes: [minute(601, [[1, 9]]), minute(602, [])],
    });
    const merged = mergeToday(held, later);
    expect(merged.minutes.map((m) => [m.m, m.s])).toEqual([
      [600, []],
      [601, [[1, 9]]],
      [602, []],
    ]);
    expect(merged.series).toHaveLength(1);
    // The clock went back an hour: an earlier minute comes again and replaces the one held, in its place.
    const back = mergeToday(merged, { ...later, minutes: [minute(600, [[1, 7]])] });
    expect(back.minutes.map((m) => [m.m, m.s])).toEqual([
      [600, [[1, 7]]],
      [601, [[1, 9]]],
      [602, []],
    ]);
    const unchanged = later.series.map((s) => ({ ...s }));
    expect(mergeToday(back, { ...later, series: unchanged }).series).toBe(back.series);
    const renamed = unchanged.map((s) => ({ ...s, name: 'A new title' }));
    expect(mergeToday(back, { ...later, series: renamed }).series).toBe(renamed);
    // Nothing new: the minutes stay.
    expect(mergeToday(held, { ...later, minutes: [] }).minutes).toHaveLength(2);
  });
});
