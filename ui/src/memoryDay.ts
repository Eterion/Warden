// A day of memory from the server, as the Memory page and the Overview's Memory card both read it: the day once
// (`GET /api/memory/day`), then, for today, each new sample through the long poll (`GET /api/memory/today/wait`),
// merged into what is held. Paused while the window is hidden or minimized, and ended when the page holding it goes
// (its effect scope ends). At midnight today becomes a new day: the caller's day follows it if it is today's.

import { computed, onScopeDispose, ref, shallowRef, watch, type Ref } from 'vue';
import { ApiError, api, type MemoryDay } from './api';
import { isoDay, mergeToday } from './memory';
import { store } from './store';

const RETRY_MS = 5000;

/** Waits before trying again; leaving the page or the day ends the wait at once. */
const sleep = (ms: number, signal: AbortSignal) =>
  new Promise<void>((resolve) => {
    const timer = setTimeout(resolve, ms);
    signal.addEventListener('abort', () => {
      clearTimeout(timer);
      resolve();
    });
  });

export interface MemoryFeed {
  /** The day read, or null while it is read (never another day's). */
  data: Ref<MemoryDay | null>;
  /** The days with recordings, kept while another day loads. */
  days: Ref<string[]>;
  /** Why the day could not be read, while it could not. */
  error: Ref<string | null>;
}

/** Reads `day` (yyyy-MM-dd) and, while it is today, follows it. */
export function useMemoryDay(day: Readonly<Ref<string>>): MemoryFeed {
  const today = computed(() => isoDay(new Date(store.now)));
  const data = shallowRef<MemoryDay | null>(null);
  const days = ref<string[]>([]);
  const error = ref<string | null>(null);
  let controller: AbortController | null = null;

  async function read(): Promise<void> {
    controller?.abort();
    const mine = new AbortController();
    controller = mine;
    const d = day.value;
    // The last day's figures are never shown as this one's.
    if (data.value?.date !== d) data.value = null;
    while (!mine.signal.aborted) {
      try {
        const answer = await api.memoryDay(d, mine.signal);
        if (mine.signal.aborted) return;
        data.value = answer;
        days.value = answer.days;
        error.value = null;
        break;
      } catch (e) {
        if (mine.signal.aborted) return;
        error.value = e instanceof ApiError ? e.message : 'Warden could not be reached.';
        if (e instanceof ApiError) return;
        await sleep(RETRY_MS, mine.signal);
      }
    }
    if (d !== today.value) return;
    let sample: number | null = null;
    while (!mine.signal.aborted) {
      const held = data.value;
      const after = held?.minutes[held.minutes.length - 1]?.m ?? 0;
      try {
        const later = await api.memoryAfter(after, sample, mine.signal);
        if (mine.signal.aborted) return;
        sample = later.sample;
        if (later.date !== d) {
          // Midnight: today is a new day. When the day asked for moves with it, the watch below reads the new one;
          // otherwise the same day is read again, whole (a past day now, or today still by this computer's clock).
          const before = day.value;
          store.now = Date.now();
          if (day.value === before) void read();
          return;
        }
        data.value = held && held.date === d ? mergeToday(held, later) : later;
        days.value = later.days;
        error.value = null;
      } catch (e) {
        if (mine.signal.aborted) return;
        if (!(e instanceof ApiError)) error.value = 'Warden could not be reached.';
        await sleep(RETRY_MS, mine.signal);
      }
    }
  }

  watch(
    [day, () => store.away],
    ([, away]) => {
      if (away) controller?.abort();
      else void read();
    },
    { immediate: true },
  );
  // Leaving the page ends its requests and waits.
  onScopeDispose(() => controller?.abort());

  return { data, days, error };
}
