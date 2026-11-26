// The Memory page's words and numbers: days, sizes, bands and figures, and the chart's geometry. Memory in KiB.

import type { Account, MemoryDay, MemoryMinute, MemoryScale, Provider } from './api';
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

export type BandKind = 'account' | 'other-sign-in' | 'left' | 'warden';

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
  /** The shown sessions that divide an open group's band. */
  slices?: Band[];
}

/** The shades each provider has; a fourth account of one provider takes the first again. */
export const SHADES = 3;

/** A band's colour: a shade of its provider for an account, a grey for each group. */
export type BandColour = `${Provider}-${1 | 2 | 3}` | 'other-sign-in' | 'left' | 'warden';

/**
 * The day's bands: main and the accounts in use always, in the Overview's order, then any other account that ran
 * something that day (switched off, or removed: named as it was then), then "Other sign-in" and "Left running" when
 * there are any. Warden itself is never a band.
 */
export function bandsOf(day: Pick<MemoryDay, 'series'>, used: Account[], all: Account[]): Band[] {
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

/** The reading for each recorded minute, with its first maximum. Indices match the day's minute array. */
interface Reading {
  values: Float64Array;
  peak: number;
  peakIndex: number;
}

export interface MemoryReadings {
  /** Whole-band and open session figures, including rows unchecked in the table. */
  rows: Map<string, Reading>;
  /** The same arrays for untouched groups, or separate arrays for partly hidden groups. */
  shown: Map<string, Reading>;
  total: Reading;
  warden: Reading | null;
}

/** Aggregate the raw samples once for the table, figures, plot and guide. Closed groups need one array each. */
export function memoryReadings(
  day: Pick<MemoryDay, 'minutes' | 'series'>,
  bands: Band[],
  visible: Band[] = bands,
  open?: ReadonlyMap<string, Band[]>,
): MemoryReadings {
  const { minutes } = day;
  function reading(): Reading {
    return { values: new Float64Array(minutes.length), peak: 0, peakIndex: minutes.length ? 0 : -1 };
  }
  interface Target {
    row?: Reading;
    partial?: Reading;
    slice?: Reading;
    shown?: true;
    warden?: true;
  }
  const targets = new Map<number, Target>();
  const rows = new Map<string, Reading>();
  const shown = new Map<string, Reading>();
  const visibleGroups = new Map(visible.map((b) => [b.key, b]));
  for (const band of bands) {
    const row = reading();
    rows.set(band.key, row);
    for (const id of band.series) targets.set(id, { row });
    const active = visibleGroups.get(band.key);
    if (active) {
      const partial = active.series.length === band.series.length ? undefined : reading();
      shown.set(band.key, partial ?? row);
      for (const id of active.series) {
        const target = targets.get(id)!;
        target.shown = true;
        target.partial = partial;
      }
    }
    for (const slice of open?.get(band.key) ?? active?.slices ?? []) {
      const detail = reading();
      rows.set(slice.key, detail);
      for (const id of slice.series) targets.get(id)!.slice = detail;
    }
  }
  const total = reading();
  let warden: Reading | null = null;
  for (const series of day.series) {
    if (series.kind !== 'warden') continue;
    warden ??= rows.get('warden') ?? reading();
    const target = targets.get(series.series);
    if (target?.row === warden) continue;
    if (target) target.warden = true;
    else targets.set(series.series, { warden: true });
  }
  // KiB values are nonnegative. A running sum can only reach its minute's maximum at its final value.
  function add(to: Reading, index: number, kib: number): void {
    const amount = (to.values[index]! += kib);
    if (amount > to.peak) {
      to.peak = amount;
      to.peakIndex = index;
    }
  }
  for (let i = 0; i < minutes.length; i++) {
    const samples = minutes[i]!.s;
    for (let j = 0; j < samples.length; j++) {
      const sample = samples[j]!;
      const target = targets.get(sample[0]);
      if (!target) continue;
      const kib = sample[1];
      if (target.row) add(target.row, i, kib);
      if (target.partial) add(target.partial, i, kib);
      if (target.slice) add(target.slice, i, kib);
      if (target.shown) add(total, i, kib);
      if (target.warden) add(warden!, i, kib);
    }
  }
  return { rows, shown, total, warden };
}

// ---- Figures ----

export interface BandRow<T extends Band = Band> {
  band: T;
  /** Today only: in the latest minute. */
  now: number | null;
  peak: number;
}

export function bandRows<T extends Band>(
  day: MemoryDay,
  bands: T[],
  readings = memoryReadings(day, bands),
): BandRow<T>[] {
  return bands.map((band) => {
    const reading = readings.rows.get(band.key)!;
    return {
      band,
      now: day.today && day.minutes.length ? reading.values.at(-1)! : null,
      peak: reading.peak,
    };
  });
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
  wardenPeak: { used: number; at: number } | null;
  rows: BandRow[];
}

/** The page's figures for the bands shown (`shown` holds their keys), and each band's row in the table. */
export function figures(
  day: MemoryDay,
  bands: Band[],
  shown: Set<string>,
  allAiShown = shown.size === bands.length,
  visible = bands.filter((b) => shown.has(b.key)),
  readings = memoryReadings(day, bands, visible),
): Figures {
  const { total, warden } = readings;
  const last = day.minutes.length - 1;
  const now = day.today && last >= 0 ? total.values[last]! : null;
  let usedIndex = last;
  if (!day.today) {
    usedIndex = -1;
    for (let i = 0; i <= last; i++) {
      if (usedIndex < 0 || day.minutes[i]!.used > day.minutes[usedIndex]!.used) usedIndex = i;
    }
  }
  const computer = day.minutes[usedIndex];
  const peak = allAiShown && day.summary.peak !== null ? day.summary.peak : last >= 0 ? total.peak : null;
  const peakAt =
    allAiShown && day.summary.peak !== null ? day.summary.peakAt : (day.minutes[total.peakIndex]?.m ?? null);
  return {
    now,
    nowShare: now === null ? '' : shareText(now, day.summary.computerTotal),
    peak,
    peakAt,
    computer: computer?.total
      ? {
          used: computer.used,
          total: computer.total,
          high: computer.used / computer.total > 0.9,
          ai: total.values[usedIndex]!,
          at: computer.m,
        }
      : null,
    warden: warden ? (day.today && last >= 0 ? warden.values[last]! : warden.peak) : null,
    wardenPeak:
      warden && warden.peakIndex >= 0
        ? {
            used: warden.peak,
            at: day.minutes[warden.peakIndex]!.m,
          }
        : null,
    rows: bandRows(day, bands, readings),
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
/** The Overview's small chart (its Memory card): shorter, shorter ticks, and no gridline labels. */
export const CHART_SMALL: ChartSize = { height: 84, gutter: 0, axis: 20, top: 6, tick: 4, line: 16 };

export interface ChartModel {
  /** The minutes across, from the day's first recording to its last (on the Memory page at least an hour): [from, to). */
  from: number;
  to: number;
  /** The recorded minutes, in order. */
  minutes: MemoryMinute[];
  readings: MemoryReadings;
  /** The shown bands, bottom first. */
  bands: Band[];
  /** The top of the scale (KiB) and its gridlines (three; two on the Overview's small chart). */
  top: number;
  grid: { kib: number; label: string }[];
  gaps: { from: number; to: number }[];
  computer?: boolean;
}

/** Gridline steps in GB: `lines` of the smallest that reaches the peak make the scale. */
const STEPS = [0.1, 0.2, 0.25, 0.5, 1, 2, 2.5, 5, 10, 20, 25, 50, 100];

/**
 * The chart of a day: its span the recorded minutes, widened to `least` (an hour on the Memory page; the Overview's
 * small chart spans only what was recorded, `least` 1), and `lines` gridlines.
 */
export function chartModel(
  day: MemoryDay,
  bands: Band[],
  shown: Set<string>,
  lines: 2 | 3 = 3,
  least = 60,
  scale: MemoryScale = 'sessions',
  readings?: MemoryReadings,
): ChartModel | null {
  const first = day.minutes[0];
  const last = day.minutes[day.minutes.length - 1];
  if (!first || !last) return null;
  let from = first.m;
  let to = last.m + 1;
  if (to - from < least) {
    to = Math.min(1440, from + least);
    from = Math.max(0, to - least);
  }
  const visible = bands.filter((b) => shown.has(b.key));
  readings ??= memoryReadings(day, visible);
  const rows = readings.rows;
  const ordered = visible.map((band) =>
    band.slices?.length
      ? { ...band, slices: [...band.slices].sort((a, b) => rows.get(b.key)!.peak - rows.get(a.key)!.peak) }
      : band,
  );
  const computer = scale === 'computer' && last.total > 0;
  let top = last.total;
  let step: number;
  if (computer) {
    const room = top / GIB / lines;
    step = room / 2;
    for (const round of STEPS) {
      if (round >= room) break;
      step = round;
    }
  } else {
    const peakGb = readings.total.peak / GIB;
    step = STEPS.find((s) => s * lines >= peakGb) ?? Math.ceil(peakGb / lines);
    top = step * lines * GIB;
  }
  return {
    from,
    to,
    minutes: day.minutes,
    readings,
    bands: ordered,
    top,
    grid: Array.from({ length: lines }, (_, i) => {
      const kib = step * (i + 1) * GIB;
      return { kib, label: `${GRID.format(kib / GIB)} GB` };
    }),
    gaps: day.gaps,
    computer,
  };
}

/** Computer memory in use over the stack, broken where nothing was recorded. */
export function computerPath(model: ChartModel, width: number, chart: ChartSize): string {
  if (!model.computer) return '';
  const x = (m: number) => ((m - model.from) / (model.to - model.from)) * width;
  const y = (kib: number) => chart.top + (1 - Math.min(kib, model.top) / model.top) * (chart.height - chart.top);
  let path = '';
  let last = -2;
  for (const minute of model.minutes) {
    const height = y(minute.used);
    if (minute.m !== last + 1) {
      if (last >= 0) path += `H${x(last + 1)}`;
      path += `M${x(minute.m)},${height}`;
    }
    path += `L${x(minute.m + 0.5)},${height}`;
    last = minute.m;
  }
  return last >= 0 ? `${path}H${x(last + 1)}` : '';
}

/** A trend over the recorded span in a 100 by 24 view box, from the same readings as the main chart. */
export function memoryTrend(minutes: MemoryMinute[], reading: Reading) {
  const first = minutes[0];
  const last = minutes.at(-1);
  if (!first || !last) return null;
  const from = first.m;
  const span = last.m + 1 - from;
  const peak = Math.max(1, reading.peak);
  const x = (m: number) => (((m - from) / span) * 100).toFixed(2);
  const y = (kib: number) => (22 - (kib / peak) * 20).toFixed(2);
  let line = '';
  let area = '';
  let segment = '';
  let start = '';
  let prev = -2;
  function finishRun(end: number): void {
    const run = `${segment}H${x(end)}`;
    line += run;
    area += `${run}V24H${start}Z`;
  }
  for (let i = 0; i < minutes.length; i++) {
    const minute = minutes[i]!.m;
    const height = y(reading.values[i]!);
    if (minute !== prev + 1) {
      if (prev >= 0) finishRun(prev + 1);
      start = x(minute);
      segment = `M${start},${height}`;
    }
    segment += `L${x(minute + 0.5)},${height}`;
    prev = minute;
  }
  finishRun(prev + 1);
  return { line, area };
}

interface BandPath {
  key: string;
  group?: string;
  colour: BandColour;
  area: string;
  edge: string;
}

/** A step per minute, joining flat runs and reusing the top edge in the filled path. */
export function bandPaths(model: ChartModel, width: number, chart: ChartSize = CHART): BandPath[] {
  const { minutes, readings } = model;
  const span = model.to - model.from;
  const round = (v: number) => Math.round(v * 100) / 100;
  const x = (m: number) => round(((m - model.from) / span) * width);
  const y = (kib: number) => round(chart.top + (1 - Math.min(kib, model.top) / model.top) * (chart.height - chart.top));
  const base = new Float64Array(minutes.length);
  const top = new Float64Array(minutes.length);
  const divided = model.bands.some((b) => b.slices?.length);
  const sliceBase = divided ? new Float64Array(minutes.length) : base;
  const sliceTop = divided ? new Float64Array(minutes.length) : top;
  const paths: BandPath[] = [];

  function stack(low: Float64Array, values: Float64Array, high: Float64Array): void {
    for (let i = 0; i < minutes.length; i++) high[i] = low[i]! + values[i]!;
  }
  function steps(values: Float64Array, first: number, last: number, direction: 1 | -1): string {
    const offset = direction === 1 ? 0 : 1;
    let height = y(values[first]!);
    let path = `${direction === 1 ? 'M' : 'L'}${x(minutes[first]!.m + offset)},${height}`;
    for (let i = first; i !== last;) {
      i += direction;
      const next = y(values[i]!);
      if (height !== next) {
        path += `H${x(minutes[i]!.m + offset)}V${next}`;
        height = next;
      }
    }
    return `${path}H${x(minutes[last]!.m + 1 - offset)}`;
  }
  function draw(band: Band, reading: Reading, low: Float64Array, high: Float64Array, group?: string): void {
    let area = '';
    let edge = '';
    let i = 0;
    while (i < minutes.length) {
      if (group && !reading.values[i]) {
        i++;
        continue;
      }
      const first = i++;
      while (i < minutes.length && minutes[i]!.m === minutes[i - 1]!.m + 1 && (!group || reading.values[i])) i++;
      const last = i - 1;
      const upper = steps(high, first, last, 1);
      area += `${upper}${steps(low, last, first, -1)}Z`;
      if (!group) {
        edge += upper;
        continue;
      }
      // A slice draws only the interior divider. Its group's edge already covers the outside.
      let j = first;
      while (j <= last) {
        if (high[j]! >= top[j]!) {
          j++;
          continue;
        }
        const start = j++;
        while (j <= last && high[j]! < top[j]!) j++;
        edge += steps(high, start, j - 1, 1);
      }
    }
    paths.push({ key: band.key, group, colour: band.colour, area, edge });
  }

  for (const band of model.bands) {
    const reading = readings.shown.get(band.key)!;
    stack(base, reading.values, top);
    draw(band, reading, base, top);
    if (band.slices?.length) {
      sliceBase.set(base);
      for (const slice of band.slices) {
        const values = readings.rows.get(slice.key)!;
        if (!values.peak) continue;
        stack(sliceBase, values.values, sliceTop);
        draw(slice, values, sliceBase, sliceTop, band.key);
        sliceBase.set(sliceTop);
      }
    }
    base.set(top);
  }
  return paths;
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
 * the plot, the first recorded minute at the left and the last at the right (today, the latest sample's); one label
 * when only one minute was recorded.
 */
export function endAxis(model: ChartModel, width: number): Axis {
  const last = model.to - 1;
  return {
    step: model.to - model.from,
    labels: [
      { m: model.from, x: 0, label: clock(model.from), anchor: 'start' },
      ...(last > model.from ? [{ m: last, x: width, label: clock(last), anchor: 'end' as const }] : []),
    ],
    ticks: [
      { m: model.from, x: 0, major: true },
      { m: model.to, x: width, major: true },
    ],
  };
}

interface GuideReading {
  key: string;
  label: string;
  kib: number;
}

export interface Guide {
  time: string;
  rows: (GuideReading & {
    sessions?: GuideReading[];
    more?: { count: number; kib: number };
  })[];
  total: number;
  computer?: number;
  /** A minute with no recording: Warden was not running (a gap), or nothing was recorded. */
  missing: 'not-running' | 'nothing' | null;
}

/** Groups with memory and up to five sessions from each open band at the chosen minute. */
export function guideAt(model: ChartModel, m: number): Guide {
  const index = model.minutes.findIndex((minute) => minute.m === m);
  if (index < 0) {
    const gap = model.gaps.some((g) => m >= g.from && m < g.to);
    return { time: clock(m), rows: [], total: 0, missing: gap ? 'not-running' : 'nothing' };
  }
  const { readings } = model;
  const entry = (band: Band, reading: Reading): GuideReading => ({
    key: band.key,
    label: band.label,
    kib: reading.values[index]!,
  });
  const largest = (a: GuideReading, b: GuideReading) => b.kib - a.kib;
  const rows = model.bands
    .filter((band) => readings.shown.get(band.key)!.values[index]! > 0)
    .map((band) => {
      const row = entry(band, readings.shown.get(band.key)!);
      if (!band.slices?.length) return row;
      const sessions = band.slices
        .map((slice) => entry(slice, readings.rows.get(slice.key)!))
        .filter((s) => s.kib > 0)
        .sort(largest);
      const more = sessions.slice(5);
      return {
        ...row,
        sessions: sessions.slice(0, 5),
        ...(more.length ? { more: { count: more.length, kib: more.reduce((sum, r) => sum + r.kib, 0) } } : {}),
      };
    })
    .sort(largest);
  return {
    time: clock(m),
    rows,
    total: readings.total.values[index]!,
    ...(model.computer ? { computer: model.minutes[index]!.used } : {}),
    missing: null,
  };
}

export const MISSING_TEXT = { 'not-running': 'Warden not running', nothing: 'Nothing recorded' } as const;

/** The guide's words for assistive technology, including which group contains each session. */
export function guideText(g: Guide): string {
  if (g.missing) return `${g.time}: ${MISSING_TEXT[g.missing]}`;
  const parts = g.rows.flatMap((r) => {
    const lines = [`${r.label} ${size(r.kib)}`];
    for (const s of r.sessions ?? []) lines.push(`${s.label} under ${r.label} ${size(s.kib)}`);
    if (r.more) lines.push(`${r.more.count} more, ${size(r.more.kib)} included in ${r.label}`);
    return lines;
  });
  parts.push(`total ${size(g.total)}`);
  if (g.computer !== undefined) parts.push(`Computer in use ${size(g.computer)}`);
  return `${g.time}: ${parts.join(', ')}`;
}

/**
 * Merges a later answer of today into what the page holds: each minute it brings replaces the one held (an earlier
 * minute too, when the clock went back an hour), the rest stay, in order.
 */
export function mergeToday(day: MemoryDay, later: MemoryDay): MemoryDay {
  let minutes = day.minutes;
  if (later.minutes.length) {
    // The API supplies both lists in order. A new reading wins when the lists name the same minute.
    minutes = [];
    let before = 0;
    let after = 0;
    while (before < day.minutes.length || after < later.minutes.length) {
      const held = day.minutes[before];
      const update = later.minutes[after];
      if (!update || (held && held.m < update.m)) {
        minutes.push(held!);
        before++;
      } else {
        minutes.push(update);
        after++;
        if (held?.m === update.m) before++;
      }
    }
  }
  const sameSeries =
    day.series.length === later.series.length &&
    later.series.every((s, i) => {
      const old = day.series[i]!;
      return (
        s.series === old.series &&
        s.kind === old.kind &&
        s.provider === old.provider &&
        s.accountId === old.accountId &&
        s.account === old.account &&
        s.sessionId === old.sessionId &&
        s.parentSessionId === old.parentSessionId &&
        s.name === old.name &&
        s.project === old.project
      );
    });
  return { ...later, minutes, series: sameSeries ? day.series : later.series };
}
