import { describe, expect, it } from 'vitest';
import type { MemoryDay, MemoryMinute, MemorySeries } from './api';
import {
  bandPaths,
  bandsOf,
  chartModel,
  dayTitle,
  figures,
  guideAt,
  guideText,
  hourTicks,
  mergeToday,
  shareText,
  size,
  stepDays,
} from './memory';
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
    expect(bands.map((b) => b.tone)).toEqual([1, 2, 3, 4, 5, 1]);
    expect(bands[3]!.account).toEqual({ id: 'gone', provider: 'claude', label: 'Old account' });
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

  it('marks the computer above 90% in use', () => {
    expect(figures(d, bands, new Set()).computer).toEqual({ used: 30 * GIB, total: 32 * GIB, high: true });
    const calm = { ...d, minutes: [minute(600, [[1, GIB]], 28 * GIB)] };
    expect(figures(calm, bands, new Set()).computer?.high).toBe(false);
  });

  it('on a past day, gives no now, and the computer and Warden at their highest', () => {
    const past = { ...d, today: false };
    const f = figures(past, bands, new Set(['account:main']));
    expect(f.now).toBeNull();
    expect(f.rows[0]!.now).toBeNull();
    expect(f.computer?.used).toBe(30 * GIB);
    expect(f.warden).toBe(9 * MIB);
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
    expect(mainPath!.area.startsWith('M0,136L1,136L1,136L2,136L2,200L1,200L1,200L0,200Z')).toBe(true);
    expect(workPath!.area).toContain('M0,72L1,72L1,136L2,136');
    expect(workPath!.area).toContain('M100,72L101,72L101,200L100,200Z');
    // Left out, a band is not drawn and the others close up.
    const alone = bandPaths(chartModel(d, bands, new Set(['account:a1']))!, 101);
    expect(alone.map((p) => p.key)).toEqual(['account:a1']);
  });

  it('marks the hours, as many as fit', () => {
    const model = chartModel({ ...d, minutes: [minute(480, []), minute(1080, [])], gaps: [] }, bands, both)!;
    expect(hourTicks(model, 800).map((t) => t.label)).toEqual([
      '08:00',
      '09:00',
      '10:00',
      '11:00',
      '12:00',
      '13:00',
      '14:00',
      '15:00',
      '16:00',
      '17:00',
      '18:00',
    ]);
    expect(hourTicks(model, 400).map((t) => t.label)).toEqual(['08:00', '10:00', '12:00', '14:00', '16:00', '18:00']);
  });

  it('says what the guide shows: the bands that ran that day, top first, and their total', () => {
    const model = chartModel(d, bands, both)!;
    const g = guideAt(model, d.minutes[1]!, new Set(['account:main', 'account:a1']));
    expect(g).toEqual({
      time: '10:01',
      rows: [
        { key: 'account:a1', label: 'Work', kib: 0 },
        { key: 'account:main', label: 'Main', kib: GIB },
      ],
      total: GIB,
    });
    expect(guideText(g)).toBe('10:01: Work 0 MB, Main 1 GB, total 1 GB');
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
    // Nothing new: the minutes stay.
    expect(mergeToday(held, { ...later, minutes: [] }).minutes).toHaveLength(2);
  });
});
