// Memory's account bands, filter choices and session metadata, shared across samples of the same day.
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
  const grouped = bandsOf({ series }, used, all);
  const accountBands = grouped.filter((b) => b.kind !== 'left');
  const accounts = new Map(accountBands.map((b) => [b.account?.id ?? OTHER_SIGN_IN, b]));
  const counts = new Map<string, number>();
  const sessions = new Set<string>();
  const warden: Band = {
    key: 'warden',
    kind: 'warden',
    colour: 'warden',
    label: 'Warden itself',
    account: null,
    series: [],
  };
  for (const s of series) {
    if (s.kind === 'warden') {
      warden.series.push(s.series);
      continue;
    }
    const id = accountOf(s);
    if (!accounts.has(id)) {
      const known = all.find((a) => a.id === id);
      const label = id === OTHER_SIGN_IN ? 'Other sign-in' : (known?.label ?? s.account ?? 'Removed account');
      const account = id === OTHER_SIGN_IN ? null : { id, label, provider: known?.provider ?? s.provider ?? 'claude' };
      accounts.set(id, {
        key: account ? `account:${id}` : OTHER_SIGN_IN,
        kind: account ? 'account' : 'other-sign-in',
        label,
        account,
        series: [],
        colour: 'other-sign-in',
      });
    }
    const key = `${id}:${s.provider}:${s.sessionId ?? s.series}`;
    if (!sessions.has(key)) {
      sessions.add(key);
      counts.set(id, (counts.get(id) ?? 0) + 1);
    }
  }
  // Resolve delegation against the day's recordings even when the filter leaves the parent out.
  const parents = new Map<string, MemorySeries>();
  for (const s of series) {
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
  const left = grouped.find((b) => b.kind === 'left');
  if (!accounts.has(OTHER_SIGN_IN)) {
    accounts.set(OTHER_SIGN_IN, {
      key: OTHER_SIGN_IN,
      kind: 'other-sign-in',
      label: 'Other sign-in',
      account: null,
      colour: 'other-sign-in',
      series: [],
    });
  }
  const choices = [...accounts].map(([id, band]) => ({ id, band, count: counts.get(id) ?? 0 }));
  return { accountBands, left, warden, choices, parents, byId: new Map(series.map((s) => [s.series, s])) };
}

export function memoryGroups(catalog: ReturnType<typeof memoryCatalog>, filter: MemoryFilter) {
  const { accountBands, left, warden, choices, byId } = catalog;
  const hidden = new Set(filter.hiddenAccounts);
  const bands = accountBands.filter((b) => !hidden.has(b.account?.id ?? OTHER_SIGN_IN));
  if (left && filter.showLeft) {
    const series = left.series.filter((id) => !hidden.has(accountOf(byId.get(id)!)));
    if (series.length) bands.push({ ...left, series });
  }
  if (filter.showWarden && warden.series.length) bands.push(warden);

  const hiddenCount = choices.filter(({ id }) => hidden.has(id)).length + Number(!filter.showLeft);
  return {
    bands,
    choices,
    hiddenCount,
    leftCount: left?.series.length ?? 0,
    // With no AI recordings, the default filter leaves the quiet-day state in place.
    allHidden:
      !bands.some((b) => b.series.length) &&
      (!!left || accountBands.some((b) => b.series.length > 0) || choices.every(({ id }) => hidden.has(id))),
  };
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
