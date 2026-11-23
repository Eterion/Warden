// The Memory page's words and numbers: days as the page names them and the nearest recorded ones, sizes, the bands
// (one per account, then "Other sign-in" and "Left running"; Warden itself is never one), the figures for what is
// shown, and the chart's geometry. English whatever the Windows region is, on a 24-hour clock. Memory in KiB.

import type { Account, MemoryDay, MemoryMinute, Provider } from './api';
import { clock } from './format';

// ---- Days ----

/** A local date as yyyy-MM-dd. */
export function isoDay(d: Date): string {
  const two = (n: number) => String(n).padStart(2, '0');
  return `${d.getFullYear()}-${two(d.getMonth() + 1)}-${two(d.getDate())}`;
}

/** yyyy-MM-dd as a local date at noon (away from daylight-saving edges). */
function parseDay(day: string): Date {
  const [y, m, d] = day.split('-').map(Number);
  return new Date(y ?? 1970, (m ?? 1) - 1, d ?? 1, 12);
}

const DAY_PARTS = new Intl.DateTimeFormat('en', { weekday: 'long', day: 'numeric', month: 'long', year: 'numeric' });

/** "Monday, 23 November", with the year when it is not this one's. */
export function dayTitle(day: string, today: string): string {
  const parts = DAY_PARTS.formatToParts(parseDay(day));
  const part = (t: string) => parts.find((p) => p.type === t)?.value ?? '';
  const year = day.slice(0, 4) === today.slice(0, 4) ? '' : ` ${part('year')}`;
  return `${part('weekday')}, ${part('day')} ${part('month')}${year}`;
}

/**
 * The days Previous and Next go to: the nearest day with recordings before and after `day`, never past today (Next
 * from the last recorded day before today goes to today). `null`: there is none.
 */
export function stepDays(days: string[], day: string, today: string): { prev: string | null; next: string | null } {
  const before = days.filter((d) => d < day);
  const after = days.filter((d) => d > day && d < today);
  return {
    prev: before.length ? before[before.length - 1]! : null,
    next: day >= today ? null : (after[0] ?? today),
  };
}

// ---- Sizes ----

const MIB = 1024;
const GIB = 1024 * 1024;
const GB = new Intl.NumberFormat('en', { maximumFractionDigits: 1 });
const MB = new Intl.NumberFormat('en', { maximumFractionDigits: 0 });
const GRID = new Intl.NumberFormat('en', { maximumFractionDigits: 2 });

/** Memory as the page writes it: GB with one decimal from 1 GB ("3.4 GB", "32 GB"), whole MB below it ("812 MB"). */
export function size(kib: number): string {
  return kib >= GIB ? `${GB.format(kib / GIB)} GB` : `${MB.format(kib / MIB)} MB`;
}

/** A share of the computer's memory: "11%", "<1%" for a little, "0%" for none. */
export function shareText(kib: number, total: number | null): string {
  if (!total) return '';
  const pct = (kib / total) * 100;
  return pct > 0 && pct < 0.5 ? '<1%' : `${Math.round(pct)}%`;
}

// ---- Bands ----

export type BandKind = 'account' | 'other-sign-in' | 'left';

export interface Band {
  key: string;
  kind: BandKind;
  label: string;
  /** The account, as the table shows its icon; null for the groups. */
  account: { id: string; provider: Provider; label: string } | null;
  /** The day's series it adds up. */
  series: number[];
  /** Its colour, which follows the account whatever is shown (`bandColours`). */
  colour: BandColour;
}

/** The shades each provider has; a fourth account of one provider takes the first again. */
export const SHADES = 3;

/** A band's colour: a shade of its provider for an account, a grey for each group. */
export type BandColour = `${Provider}-${1 | 2 | 3}` | 'other-sign-in' | 'left';

/**
 * The day's bands: main and the accounts in use always, in the Overview's order, then any other account that ran
 * something that day (switched off, or removed: named as it was then), then "Other sign-in" and "Left running" when
 * there are any. Warden itself is never a band.
 */
export function bandsOf(day: MemoryDay, used: Account[], all: Account[]): Band[] {
  const bands: Band[] = [];
  const byKey = new Map<string, Band>();
  const add = (band: Omit<Band, 'colour'>) => {
    const b: Band = { ...band, colour: 'claude-1' };
    bands.push(b);
    byKey.set(b.key, b);
    return b;
  };
  for (const a of used)
    add({
      key: `account:${a.id}`,
      kind: 'account',
      label: a.label,
      account: { id: a.id, provider: a.provider, label: a.label },
      series: [],
    });
  let other: Band | null = null;
  let left: Band | null = null;
  for (const s of day.series) {
    if (s.kind === 'warden') continue;
    if (s.kind === 'left') {
      left ??= { key: 'left', kind: 'left', label: 'Left running', account: null, series: [], colour: 'left' };
      left.series.push(s.series);
      continue;
    }
    if (s.kind === 'other-sign-in' || !s.accountId) {
      other ??= {
        key: 'other-sign-in',
        kind: 'other-sign-in',
        label: 'Other sign-in',
        account: null,
        series: [],
        colour: 'other-sign-in',
      };
      other.series.push(s.series);
      continue;
    }
    let band = byKey.get(`account:${s.accountId}`);
    if (!band) {
      const known = all.find((a) => a.id === s.accountId);
      const label = known?.label ?? s.account ?? 'Removed account';
      band = add({
        key: `account:${s.accountId}`,
        kind: 'account',
        label,
        account: { id: s.accountId, provider: known?.provider ?? s.provider ?? 'claude', label },
        series: [],
      });
    }
    band.series.push(s.series);
  }
  if (other) bands.push(other);
  if (left) bands.push(left);
  const colours = bandColours(
    all,
    bands.flatMap((b) => (b.account ? [b.account] : [])),
  );
  for (const b of bands) if (b.account) b.colour = colours.get(b.account.id) ?? b.colour;
  return bands;
}

/**
 * Each account's colour: its provider's shades in settings order (main first, so Claude's base shade), then the
 * accounts no longer in Settings in the order they show; a fourth of one provider starts the shades again. It depends
 * only on the accounts, never on which bands are shown.
 */
export function bandColours(
  settings: { id: string; provider: Provider }[],
  others: { id: string; provider: Provider }[],
): Map<string, BandColour> {
  const colours = new Map<string, BandColour>();
  const count: Record<Provider, number> = { claude: 0, codex: 0 };
  for (const a of [...settings, ...others]) {
    if (colours.has(a.id)) continue;
    const shade = ((count[a.provider]++ % SHADES) + 1) as 1 | 2 | 3;
    colours.set(a.id, `${a.provider}-${shade}`);
  }
  return colours;
}

/** A band's memory in one minute: its series added up. */
export function bandAt(band: Band, minute: MemoryMinute): number {
  let kib = 0;
  for (const [n, v] of minute.s) if (band.series.includes(n)) kib += v;
  return kib;
}

// ---- Figures ----

export interface BandRow {
  band: Band;
  /** Today only: in the latest minute. */
  now: number | null;
  peak: number;
}

export interface Figures {
  /** Today only: the shown bands' total in the latest minute, and its share of the computer. */
  now: number | null;
  nowShare: string;
  /** The shown bands' highest total, and its minute. */
  peak: number | null;
  peakAt: number | null;
  /**
   * The computer's memory in use (today: now; a past day: at its highest, in minute `at`) of its total, `high` above
   * 90%, and the shown bands' part of it in that same minute (`ai`, the meter's mark).
   */
  computer: { used: number; total: number; high: boolean; ai: number; at: number } | null;
  /** Warden itself (today: now; a past day: at its highest). */
  warden: number | null;
  rows: BandRow[];
}

/** The page's figures for the bands shown (`shown` holds their keys), and each band's row in the table. */
export function figures(day: MemoryDay, bands: Band[], shown: Set<string>): Figures {
  const last = day.minutes[day.minutes.length - 1] ?? null;
  const rows = bands.map((band) => {
    let peak = 0;
    for (const m of day.minutes) peak = Math.max(peak, bandAt(band, m));
    return { band, now: day.today && last ? bandAt(band, last) : null, peak };
  });
  const visible = bands.filter((b) => shown.has(b.key));
  let peak: number | null = null;
  let peakAt: number | null = null;
  if (visible.length === bands.length && day.summary.peak !== null) {
    // Every band: the total Warden recorded at once, not the bands' own highs in a minute added up.
    peak = day.summary.peak;
    peakAt = day.summary.peakAt;
  } else
    for (const m of day.minutes) {
      const total = visible.reduce((sum, b) => sum + bandAt(b, m), 0);
      if (peak === null || total > peak) {
        peak = total;
        peakAt = m.m;
      }
    }
  const now = day.today && last ? visible.reduce((sum, b) => sum + bandAt(b, last), 0) : null;
  const total = day.summary.computerTotal;
  let used: number | null = null;
  let usedTotal = total;
  let usedAt: MemoryMinute | null = null;
  if (day.today && last) {
    used = last.used;
    usedTotal = last.total;
    usedAt = last;
  } else
    for (const m of day.minutes)
      if (used === null || m.used > used) {
        used = m.used;
        usedTotal = m.total;
        usedAt = m;
      }
  const wardenSeries = day.series.filter((s) => s.kind === 'warden').map((s) => s.series);
  const wardenAt = (m: MemoryMinute) => m.s.filter(([n]) => wardenSeries.includes(n)).reduce((a, [, v]) => a + v, 0);
  let warden: number | null = null;
  if (wardenSeries.length) warden = day.today && last ? wardenAt(last) : Math.max(0, ...day.minutes.map(wardenAt));
  return {
    now,
    nowShare: now === null ? '' : shareText(now, total),
    peak,
    peakAt,
    computer:
      used !== null && usedTotal && usedAt
        ? {
            used,
            total: usedTotal,
            high: used / usedTotal > 0.9,
            ai: visible.reduce((sum, b) => sum + bandAt(b, usedAt), 0),
            at: usedAt.m,
          }
        : null,
    warden,
    rows,
  };
}

/**
 * The Computer figure's meter, as shares of the computer's memory (0 to 100): the AI sessions' part of the memory in
 * use, then the rest of it.
 */
export function meterParts(c: { used: number; total: number; ai: number }): { ai: number; rest: number } {
  const pct = (kib: number) => Math.round((Math.max(0, kib) / c.total) * 10000) / 100;
  const used = Math.min(c.used, c.total);
  const ai = Math.min(c.ai, used);
  return { ai: pct(ai), rest: pct(used - ai) };
}

/** Whether any band ran something that day. */
export function anyRan(rows: BandRow[]): boolean {
  return rows.some((r) => r.peak > 0);
}

// ---- The chart ----

/** A chart's size: its plot height, the gutter for the gridlines' labels, the room under it for the time axis, a tick's length, and a line of its 12 px text. */
export interface ChartSize {
  height: number;
  gutter: number;
  axis: number;
  top: number;
  tick: number;
  line: number;
}

/** The Memory page's chart. */
export const CHART: ChartSize = { height: 200, gutter: 52, axis: 24, top: 8, tick: 6, line: 18 };
/** The Overview's small chart (its Memory card): shorter, its ticks shorter. */
export const CHART_SMALL: ChartSize = { height: 84, gutter: 52, axis: 20, top: 6, tick: 4, line: 16 };

export interface ChartModel {
  /** The minutes across, from the day's first recording to its last (at least an hour): [from, to). */
  from: number;
  to: number;
  /** The recorded minutes, in order. */
  minutes: MemoryMinute[];
  /** The shown bands, bottom first. */
  bands: Band[];
  /** The top of the scale (KiB) and its gridlines (three; two on the Overview's small chart). */
  top: number;
  grid: { kib: number; label: string }[];
  gaps: { from: number; to: number }[];
}

/** Gridline steps in GB: `lines` of the smallest that reaches the peak make the scale. */
const STEPS = [0.1, 0.2, 0.25, 0.5, 1, 2, 2.5, 5, 10, 20, 25, 50, 100];

export function chartModel(day: MemoryDay, bands: Band[], shown: Set<string>, lines: 2 | 3 = 3): ChartModel | null {
  const first = day.minutes[0];
  const last = day.minutes[day.minutes.length - 1];
  if (!first || !last) return null;
  let from = first.m;
  let to = last.m + 1;
  if (to - from < 60) {
    to = Math.min(1440, from + 60);
    from = Math.max(0, to - 60);
  }
  const visible = bands.filter((b) => shown.has(b.key));
  let peak = 0;
  for (const m of day.minutes)
    peak = Math.max(
      peak,
      visible.reduce((sum, b) => sum + bandAt(b, m), 0),
    );
  const peakGb = peak / GIB;
  const step = STEPS.find((s) => s * lines >= peakGb) ?? Math.ceil(peakGb / lines);
  const top = step * lines * GIB;
  return {
    from,
    to,
    minutes: day.minutes,
    bands: visible,
    top,
    grid: Array.from({ length: lines }, (_, i) => i + 1).map((i) => ({
      kib: step * i * GIB,
      label: `${GRID.format(step * i)} GB`,
    })),
    gaps: day.gaps,
  };
}

/** Each shown band's area, as SVG paths over a plot `width` wide: a step per minute, broken where nothing was recorded. */
export function bandPaths(
  model: ChartModel,
  width: number,
  chart: ChartSize = CHART,
): { key: string; colour: BandColour; area: string; edge: string }[] {
  const span = model.to - model.from;
  const x = (m: number) => ((m - model.from) / span) * width;
  const y = (kib: number) => chart.top + (1 - Math.min(kib, model.top) / model.top) * (chart.height - chart.top);
  // Runs of consecutive minutes.
  const runs: MemoryMinute[][] = [];
  for (const m of model.minutes) {
    const run = runs[runs.length - 1];
    const prev = run?.[run.length - 1];
    if (run && prev && m.m === prev.m + 1) run.push(m);
    else runs.push([m]);
  }
  const base = new Map<number, number>();
  const n = (v: number) => Math.round(v * 100) / 100;
  return model.bands.map((band) => {
    let area = '';
    let edge = '';
    for (const run of runs) {
      const tops: string[] = [];
      const bottoms: string[] = [];
      for (const m of run) {
        const below = base.get(m.m) ?? 0;
        const above = below + bandAt(band, m);
        base.set(m.m, above);
        tops.push(`${n(x(m.m))},${n(y(above))}`, `${n(x(m.m + 1))},${n(y(above))}`);
        bottoms.unshift(`${n(x(m.m + 1))},${n(y(below))}`, `${n(x(m.m))},${n(y(below))}`);
      }
      area += `M${tops.join('L')}L${bottoms.join('L')}Z`;
      edge += `M${tops.join('L')}`;
    }
    return { key: band.key, colour: band.colour, area, edge };
  });
}

/** The axis's label steps in minutes: a quarter of an hour to three hours. */
const LABEL_STEPS = [15, 30, 60, 120, 180];
/** At most this many labels across the span, each with this much room (an "08:00" in 12 px text and a gap). */
const MOST_LABELS = 8;
const LABEL_ROOM = 48;
/** Half a label's width: a label nearer the plot's right end than this would be cut, so it moves inward. */
const LABEL_HALF = 18;
/** The least distance between two labels' middles, so they never touch. */
const LABEL_APART = 2 * LABEL_HALF + 4;

export interface Axis {
  /** The label step in minutes; ticks fall at half of it. */
  step: number;
  /**
   * Each label's minute and where it sits (`x`): its middle, the last moved inward from its tick when it would be cut;
   * with `anchor`, its start or its end (the small chart's two ends).
   */
  labels: { m: number; x: number; label: string; anchor?: 'start' | 'end' }[];
  /** Every tick along the baseline, `major` under a label's step. */
  ticks: { m: number; x: number; major: boolean }[];
}

/**
 * The time axis under the chart, over a plot `width` wide: labels at the smallest step (15 min, 30 min, 1 h, 2 h, 3 h)
 * that puts at most eight across the span with room between them, so a short span still gets four or more; ticks at
 * half the step. A label that would be cut at the right end moves inward from its tick, left out only when that would
 * crowd the one before (the left end has the gutter beside it). In a chart too narrow for even the longest step to
 * leave room, only every other label (or fewer) is kept, so none collide.
 */
export function timeAxis(model: ChartModel, width: number): Axis {
  const span = model.to - model.from;
  const px = (min: number) => (min / span) * width;
  // Labels within the span [from, to): a whole day's ends at 21:00, never at 24:00.
  const count = (step: number) => Math.floor((model.to - 1) / step) - Math.ceil(model.from / step) + 1;
  const step =
    LABEL_STEPS.find((s) => count(s) <= MOST_LABELS && px(s) >= LABEL_ROOM) ?? LABEL_STEPS[LABEL_STEPS.length - 1]!;
  const every = Math.max(1, Math.ceil(LABEL_ROOM / Math.max(px(step), 1)));
  const x = (m: number) => ((m - model.from) / span) * width;
  const half = step / 2;
  const ticks: Axis['ticks'] = [];
  for (let m = Math.ceil(model.from / half) * half; m <= model.to; m += half)
    ticks.push({ m, x: x(m), major: m % step === 0 });
  const labels: Axis['labels'] = [];
  for (let m = Math.ceil(model.from / step) * step; m < model.to; m += step) {
    if ((m / step) % every !== 0) continue;
    const at = Math.min(x(m), width - LABEL_HALF);
    const before = labels[labels.length - 1];
    if (before && at - before.x < LABEL_APART) continue;
    labels.push({ m, x: at, label: clock(m) });
  }
  return { step, labels, ticks };
}

/**
 * The Overview's small chart's time axis, over a plot `width` wide: only its two ends, each ticked and labelled inside
 * the plot, the span's first minute at the left and its last at the right (today, the latest sample's).
 */
export function endAxis(model: ChartModel, width: number): Axis {
  const last = model.to - 1;
  return {
    step: model.to - model.from,
    labels: [
      { m: model.from, x: 0, label: clock(model.from), anchor: 'start' },
      { m: last, x: width, label: clock(last), anchor: 'end' },
    ],
    ticks: [
      { m: model.from, x: 0, major: true },
      { m: model.to, x: width, major: true },
    ],
  };
}

export interface Guide {
  time: string;
  /** Each shown band that ran that day, top first; empty in a minute with no recording. */
  rows: { key: string; label: string; kib: number }[];
  total: number;
  /** A minute with no recording: Warden was not running (a gap), or nothing was recorded (before the first). */
  missing: 'not-running' | 'nothing' | null;
}

/** What the guide says at a minute of the span: the time, each shown band and their total, or why there is none. */
export function guideAt(model: ChartModel, m: number, ran: Set<string>): Guide {
  const minute = model.minutes.find((x) => x.m === m);
  if (!minute) {
    const gap = model.gaps.some((g) => m >= g.from && m < g.to);
    return { time: clock(m), rows: [], total: 0, missing: gap ? 'not-running' : 'nothing' };
  }
  const rows = model.bands
    .filter((b) => ran.has(b.key))
    .map((b) => ({ key: b.key, label: b.label, kib: bandAt(b, minute) }))
    .reverse();
  return { time: clock(m), rows, total: rows.reduce((sum, r) => sum + r.kib, 0), missing: null };
}

export const MISSING_TEXT = { 'not-running': 'Warden not running', nothing: 'Nothing recorded' } as const;

/** The guide's words for assistive technology: "14:12: Main 2.1 GB, Claude 2 1.2 GB, total 3.3 GB". */
export function guideText(g: Guide): string {
  if (g.missing) return `${g.time}: ${MISSING_TEXT[g.missing]}`;
  const parts = g.rows.map((r) => `${r.label} ${size(r.kib)}`);
  return `${g.time}: ${[...parts, `total ${size(g.total)}`].join(', ')}`;
}

/**
 * Merges a later answer of today into what the page holds: each minute it brings replaces the one held (an earlier
 * minute too, when the clock went back an hour), the rest stay, in order.
 */
export function mergeToday(day: MemoryDay, later: MemoryDay): MemoryDay {
  const brought = new Set(later.minutes.map((m) => m.m));
  const minutes = [...day.minutes.filter((m) => !brought.has(m.m)), ...later.minutes].sort((a, b) => a.m - b.m);
  return { ...later, minutes };
}
