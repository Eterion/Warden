// Sessions and activity put into shape for the Overview. Pure functions, so they are unit tested.

import type { Bin, Session, SessionState } from './api';

const MINUTE = 60_000;
export const BIN_MS = 5 * MINUTE;
export const BINS = 24;

/** The states a session's icon can show: the four states, and "unknown" for one seen only in a CLI's list. */
export type Shown = SessionState | 'unknown';

export function shownState(s: Session): Shown {
  return s.source === 'list' ? 'unknown' : s.state;
}

/** Ended, or its state not known (seen only in a CLI's list): the sessions the table shows faded, and can hide. */
export function isQuiet(s: Session): boolean {
  const shown = shownState(s);
  return shown === 'done' || shown === 'unknown';
}

export const STATE_LABEL: Record<Shown, string> = {
  working: 'Working',
  waiting: 'Waiting for you',
  idle: 'Idle',
  done: 'Done',
  unknown: 'State not known, no status hook',
};

const ORDER: Record<Shown, number> = { waiting: 0, working: 1, idle: 2, unknown: 3, done: 4 };

/** Waiting rows first, then working, idle, not known and done; the longest running first within each. */
export function sortSessions(list: Session[]): Session[] {
  return [...list].sort((a, b) => ORDER[shownState(a)] - ORDER[shownState(b)] || a.startedAt - b.startedAt);
}

/**
 * A session's name as the table shows it: its own, else the end of its id (as the ID column shows it; Codex's ids are
 * time-ordered, so their starts repeat).
 */
export function sessionName(s: { id: string; name: string | null }): string {
  return s.name ?? `Session ${s.id.slice(-8)}`;
}

/** A row of the sessions tree. */
export interface TreeRow {
  session: Session;
  /** 0 at the top level, 1 under a top-level session, and so on. */
  depth: number;
  /** The session it sits under, or null at the top level. */
  parent: Session | null;
  /** For each level above this row's own (1 to depth − 1): whether a later sibling there still needs the line. */
  rails: boolean[];
  /** Whether this row is the last of its siblings, so its connector ends here. */
  last: boolean;
  /** Shown only for the sessions under it, as their ancestor: drawn faded and not counted. */
  faded: boolean;
}

/**
 * The sessions as a tree, flattened in display order: each session whose `parentId` names a session in the tree sits
 * under it; one whose parent is unknown or gone stays at the top level. With `from`, the ancestors of the listed
 * sessions found there join the tree in their places, faded (a filter's matches under their parents). Siblings sort as
 * `sortSessions` does at every level (waiting first), and a parent never moves for its children.
 */
export function sessionTree(list: Session[], from: Session[] = list): TreeRow[] {
  const shown = new Set(list);
  const parentIn = (ids: Map<string, Session>, s: Session): Session | null => {
    const p = s.parentId !== null ? ids.get(s.parentId) : undefined;
    return p && p !== s ? p : null;
  };
  // The listed sessions and their ancestors, up to where a loop of parents closes.
  const known = new Map([...from, ...list].map((s) => [s.id, s]));
  const inTree = new Set(list);
  for (const s of list) {
    for (let p = parentIn(known, s); p && !inTree.has(p); p = parentIn(known, p)) inTree.add(p);
  }
  const byId = new Map([...inTree].map((s) => [s.id, s]));
  const parentOf = (s: Session): Session | null => parentIn(byId, s);
  // A loop of parents (which a real link never makes) is cut where it closes: those sessions go to the top level.
  const inLoop = (s: Session): boolean => {
    const seen = new Set<Session>([s]);
    for (let p = parentOf(s); p; p = parentOf(p)) {
      if (seen.has(p)) return true;
      seen.add(p);
    }
    return false;
  };
  const children = new Map<Session | null, Session[]>();
  for (const s of inTree) {
    const p = inLoop(s) ? null : parentOf(s);
    children.set(p, [...(children.get(p) ?? []), s]);
  }
  const out: TreeRow[] = [];
  const walk = (parent: Session | null, depth: number, rails: boolean[]): void => {
    const kids = sortSessions(children.get(parent) ?? []);
    kids.forEach((s, k) => {
      const last = k === kids.length - 1;
      out.push({ session: s, depth, parent, rails, last, faded: !shown.has(s) });
      walk(s, depth + 1, depth === 0 ? [] : [...rails, !last]);
    });
  };
  walk(null, 0, []);
  return out;
}

export interface Tally {
  working: number;
  waiting: number;
  idle: number;
}

/** How many sessions are working, waiting for you and idle (finished and not-known ones are not counted). */
export function tally(list: Session[]): Tally {
  const t: Tally = { working: 0, waiting: 0, idle: 0 };
  for (const s of list) {
    const st = shownState(s);
    if (st === 'working' || st === 'waiting' || st === 'idle') t[st]++;
  }
  return t;
}

/** "Now: 1 waiting for you, 2 working", or null when there are no sessions. */
export function nowLabel(t: Tally): string | null {
  const parts = [
    t.waiting && `${t.waiting} waiting for you`,
    t.working && `${t.working} working`,
    t.idle && `${t.idle} idle`,
  ].filter(Boolean);
  return parts.length ? `Now: ${parts.join(', ')}` : null;
}

/** "38m", "1h 02m", "2h 14m". */
export function runningFor(s: Session, now: number): string {
  const end = s.endedAt ?? now;
  const minutes = Math.max(0, Math.floor((end - s.startedAt) / MINUTE));
  if (minutes < 60) return `${minutes}m`;
  return `${Math.floor(minutes / 60)}h ${String(minutes % 60).padStart(2, '0')}m`;
}

/** "1 subagent: Explore", "2 subagents: Explore, general-purpose". */
export function subagentsLabel(s: Session): string {
  const n = s.subagents.length;
  const types = s.subagents.map((a) => a.type).join(', ');
  return `${n} ${n === 1 ? 'subagent' : 'subagents'}: ${types}`;
}

/** One bin of the sparkline: sessions working, and whether one started waiting for you. */
export interface Point {
  working: number;
  waited: boolean;
}

/**
 * The last two hours, then now: 24 five-minute bins ending with the current one. Bins with no change carry on the
 * count of the bin before them.
 */
export function sparkPoints(bins: Bin[], now: number): Point[] {
  const end = now - (now % BIN_MS);
  const start = end - (BINS - 1) * BIN_MS;
  const sorted = [...bins].sort((a, b) => a.at - b.at);
  let carry = 0;
  for (const b of sorted) if (b.at < start) carry = b.last;
  const byAt = new Map(sorted.map((b) => [b.at, b]));
  const out: Point[] = [];
  for (let k = 0; k < BINS; k++) {
    const b = byAt.get(start + k * BIN_MS);
    if (b) {
      out.push({ working: b.working, waited: b.waited });
      carry = b.last;
    } else out.push({ working: carry, waited: false });
  }
  return out;
}

export const SPARK = { width: 100, height: 28, pad: 2 } as const;

export interface SparkPaths {
  line: string;
  area: string;
  dots: { x: number; y: number }[];
  /** "Main, last 2 hours: up to 2 sessions working, waited for you once" */
  label: string;
}

const round = (n: number) => Math.round(n * 10) / 10;

/** The step line, its wash and the amber dots, on a scale where `most` sessions fill the height. */
export function sparkPaths(points: Point[], most: number, name: string): SparkPaths {
  const base = SPARK.height - 3;
  const binW = (SPARK.width - 2 * SPARK.pad) / BINS;
  const x0 = (k: number) => round(SPARK.pad + k * binW);
  const x1 = (k: number) => round(SPARK.pad + (k + 1) * binW);
  const y = (v: number) => round(base - (v / Math.max(1, most)) * (base - 4));
  let line = `M${x0(0)} ${y(points[0]?.working ?? 0)} H${x1(0)}`;
  for (let k = 1; k < points.length; k++) {
    const v = points[k]!.working;
    if (v !== points[k - 1]!.working) line += ` V${y(v)}`;
    line += ` H${x1(k)}`;
  }
  const area = `${line} V${base} H${x0(0)} Z`;
  const dots = points.flatMap((p, k) => (p.waited ? [{ x: round((x0(k) + x1(k)) / 2), y: y(p.working) }] : []));
  const peak = Math.max(0, ...points.map((p) => p.working));
  const waits = dots.length;
  const label =
    `${name}, last 2 hours: ` +
    (peak ? `up to ${peak} ${peak === 1 ? 'session' : 'sessions'} working` : 'nothing working') +
    (waits ? `, waited for you ${waits === 1 ? 'once' : `${waits} times`}` : '');
  return { line, area, dots, label };
}
