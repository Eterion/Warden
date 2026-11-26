// Memory's account bands and session metadata, indexed once while the day's series stay the same.
import type { Account, MemoryDay, MemoryFilter, MemorySeries, Provider } from './api';
import { bandsOf, type Band } from './memory';
import { sessionName } from './sessions';

export const OTHER_SIGN_IN = 'other-sign-in';

export const DEFAULT_MEMORY_FILTER: MemoryFilter = { hiddenAccounts: [], showLeft: true, showWarden: false };

function accountOf(series: MemorySeries): string {
  return series.accountId ?? OTHER_SIGN_IN;
}

export interface SessionBand extends Band {
  provider: Provider | null;
  from: string | null;
}

function nameOf(s: MemorySeries): string {
  return sessionName({ id: s.sessionId ?? String(s.series), name: s.name });
}

/** Index the series without depending on the day's minute readings. */
export function memoryCatalog(series: MemorySeries[], used: Account[], all: Account[]) {
  const bands = bandsOf({ series }, used, all);
  const warden: Band = {
    key: 'warden',
    kind: 'warden',
    colour: 'warden',
    label: 'Warden itself',
    account: null,
    series: [],
  };
  // Resolve delegation against the day's recordings even when the filter leaves the parent out.
  const parents = new Map<string, MemorySeries>();
  const byId = new Map<number, MemorySeries>();
  for (const s of series) {
    byId.set(s.series, s);
    if (s.kind === 'warden') warden.series.push(s.series);
    if (!s.sessionId) continue;
    const previous = parents.get(s.sessionId);
    if (
      !previous ||
      (s.provider === 'claude' && previous.provider !== 'claude') ||
      (s.provider === previous.provider && previous.kind === 'left')
    ) {
      parents.set(s.sessionId, s);
    }
  }
  bands.push(warden);
  return { bands, parents, byId };
}

/** An excluded account's left-running sessions stay out even while Left running is checked. */
export function memoryGroups(catalog: ReturnType<typeof memoryCatalog>, hiddenAccounts: string[]): Band[] {
  if (!hiddenAccounts.length) return catalog.bands;
  const hidden = new Set(hiddenAccounts);
  return catalog.bands.map((band) =>
    band.kind === 'left'
      ? { ...band, series: band.series.filter((id) => !hidden.has(accountOf(catalog.byId.get(id)!))) }
      : band,
  );
}

export function groupIsShown(group: Band, filter: MemoryFilter): boolean {
  if (group.kind === 'left') return filter.showLeft;
  if (group.kind === 'warden') return filter.showWarden;
  return !filter.hiddenAccounts.includes(group.account?.id ?? OTHER_SIGN_IN);
}

/** Stable across days for real sessions, even while a group is closed. */
export function sliceKey(group: Band, s: MemorySeries, day: string): string {
  return `${group.key}:${s.provider}:${s.sessionId ?? `${day}:${s.series}`}`;
}

export function sessionBands(catalog: ReturnType<typeof memoryCatalog>, group: Band, day: string): SessionBand[] {
  if (group.kind === 'warden') return [];
  return group.series.map((id) => {
    const s = catalog.byId.get(id)!;
    const parent = s.parentSessionId ? catalog.parents.get(s.parentSessionId) : undefined;
    return {
      key: sliceKey(group, s, day),
      kind: group.kind,
      colour: group.colour,
      account: group.account,
      label: nameOf(s),
      provider: s.provider,
      from: parent && parent.sessionId !== s.sessionId ? nameOf(parent) : null,
      series: [id],
    };
  });
}

/** Whether the server's recorded AI peak applies to this selection of displayed series. */
export function showsRecordedPeak(day: MemoryDay, bands: Band[]): boolean {
  const series = new Set(bands.flatMap((b) => b.series));
  return day.series.every((s) => series.has(s.series) === (s.kind !== 'warden'));
}
