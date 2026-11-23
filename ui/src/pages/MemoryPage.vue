<script setup lang="ts">
// The Memory page: one day at a time, its own address (#/memory is today, #/memory/<yyyy-MM-dd> a past day), so back
// and forward go through the days seen. Previous and Next go to the nearest day with recordings (Next stops at today,
// saying why); between them the day opens a calendar to pick any day (MemoryDayPicker). The figures (MemoryFigures:
// the AI sessions now and their share of the computer, today only; the peak and its time; the computer's memory in use
// with its meter, amber above 90%; Warden itself), then the chart of the AI sessions' memory stacked by account, on the
// panel without a box, and the table: the chart's legend and accessible equivalent, a checkbox per row leaving its band out (on this page only),
// hovering a row darkening its band. Today's page follows each new sample through its own long poll, paused while the
// window is hidden; a past day is read once.
import { Check, ChevronLeft, ChevronRight } from 'lucide-vue-next';
import { computed, onScopeDispose, ref, shallowRef, watch } from 'vue';
import { ApiError, api, type MemoryDay } from '../api';
import AccountIcon from '../components/AccountIcon.vue';
import WButton from '../components/WButton.vue';
import WIconButton from '../components/WIconButton.vue';
import WTable from '../components/WTable.vue';
import WTableRow from '../components/WTableRow.vue';
import {
  anyRan,
  bandsOf,
  chartModel,
  figures,
  isoDay,
  mergeToday,
  shareText,
  size,
  stepDays,
  type BandColour,
} from '../memory';
import { go, route } from '../route';
import { store, usedAccounts } from '../store';
import MemoryChart from './MemoryChart.vue';
import MemoryDayPicker from './MemoryDayPicker.vue';
import MemoryFigures from './MemoryFigures.vue';

const today = computed(() => isoDay(new Date(store.now)));
/** The day shown: the address's, never one to come. */
const day = computed(() => {
  const d = route.day.value;
  return d && d < today.value ? d : today.value;
});
const isToday = computed(() => day.value === today.value);

function show(d: string): void {
  go(d === today.value ? 'memory' : `memory/${d}`);
}

// ---- The day, from the server ----

const data = shallowRef<MemoryDay | null>(null);
const error = ref<string | null>(null);
let controller: AbortController | null = null;
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
/** The days with recordings, kept while another day loads so Previous and Next stay as they were. */
const days = ref<string[]>([]);

/** Reads the day shown, then (today, while the window is in sight) follows each new sample. */
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
        // Midnight: today is a new day; the address follows it (#/memory) and it is read whole.
        store.now = Date.now();
        if (route.day.value === null) void read();
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

// ---- What is shown ----

const all = computed(() =>
  store.state ? [...(store.state.main ? [store.state.main] : []), ...store.state.accounts] : [],
);
const bands = computed(() => (data.value ? bandsOf(data.value, usedAccounts(store.state), all.value) : []));
/** The bands left out of the chart (on this page only, not remembered). */
const hidden = ref<string[]>([]);
const shown = computed(() => new Set(bands.value.filter((b) => !hidden.value.includes(b.key)).map((b) => b.key)));
const figs = computed(() => (data.value ? figures(data.value, bands.value, shown.value) : null));
const ran = computed(() => new Set((figs.value?.rows ?? []).filter((r) => r.peak > 0).map((r) => r.band.key)));
const model = computed(() => (data.value ? chartModel(data.value, bands.value, shown.value) : null));
const steps = computed(() => stepDays(days.value, day.value, today.value));
const nothing = computed(() => !!data.value && data.value.minutes.length === 0 && !isToday.value);
/** Today, nothing ran all day: the message takes the chart's and the table's place. */
const noSessions = computed(() => !!data.value && isToday.value && !anyRan(figs.value?.rows ?? []));
/** Today, nothing runs now (some did earlier): the message shows above the day's chart. */
const noneNow = computed(
  () => !!data.value && isToday.value && !noSessions.value && (figs.value?.rows ?? []).every((r) => !r.now),
);
const total = computed(() => data.value?.summary.computerTotal ?? null);
const hovered = ref<string | null>(null);
const SWATCH: Record<BandColour, string> = {
  'claude-1': 'bg-chart-claude-1',
  'claude-2': 'bg-chart-claude-2',
  'claude-3': 'bg-chart-claude-3',
  'codex-1': 'bg-chart-codex-1',
  'codex-2': 'bg-chart-codex-2',
  'codex-3': 'bg-chart-codex-3',
  'other-sign-in': 'bg-chart-other-sign-in',
  left: 'bg-chart-left',
};

function toggle(key: string): void {
  hidden.value = hidden.value.includes(key) ? hidden.value.filter((k) => k !== key) : [...hidden.value, key];
}
</script>

<template>
  <div class="flex flex-col gap-4.5">
    <header class="flex flex-wrap items-center gap-x-3 gap-y-2">
      <h1 class="text-20 font-semibold">Memory</h1>
      <div class="ml-auto flex flex-wrap items-center gap-x-2 gap-y-2">
        <WIconButton
          label="Previous day"
          :icon="ChevronLeft"
          :disabled-reason="steps.prev ? null : 'No earlier day was recorded.'"
          @click="steps.prev && show(steps.prev)"
        />
        <h2><MemoryDayPicker :day="day" :today="today" :recorded="days" @pick="show" /></h2>
        <WIconButton
          label="Next day"
          :icon="ChevronRight"
          :disabled-reason="steps.next ? null : 'This is today: there is no later day.'"
          @click="steps.next && show(steps.next)"
        />
        <WButton variant="quiet" :disabled-reason="isToday ? 'This is today.' : null" @click="show(today)"
          >Today</WButton
        >
      </div>
    </header>

    <p v-if="error && !data" class="text-14 text-secondary">
      {{ store.connected ? error : "Warden's core is not running. Start it, and this page fills in by itself." }}
    </p>
    <p v-else-if="!data" class="text-14 text-secondary">Reading the day…</p>
    <div
      v-else-if="nothing"
      class="flex flex-col items-start rounded-lg border border-divider-strong p-4 text-14 text-tertiary"
      data-memory-empty
    >
      Nothing recorded on this day.
    </div>
    <template v-else-if="figs">
      <MemoryFigures :figures="figs" :today="isToday" :total="total" />

      <div
        v-if="noSessions"
        class="flex flex-col items-start rounded-lg border border-divider-strong p-4 text-14 text-tertiary"
        data-memory-none
      >
        No AI sessions are running.
      </div>
      <template v-else>
        <p v-if="noneNow" class="text-14 text-tertiary" data-memory-none-now>No AI sessions are running.</p>
        <section aria-label="Memory over the day">
          <MemoryChart v-if="model" :model="model" :ran="ran" :hovered="hovered" :day="day" />
        </section>

        <WTable
          on="panel"
          aria-label="Memory by account"
          cols="grid-cols-memory gap-x-4"
          class="min-w-memory"
          data-memory-table
        >
          <template #head>
            <span role="columnheader"><span class="sr-only">Shown in the chart</span></span>
            <span role="columnheader">Account</span>
            <span v-if="isToday" role="columnheader" class="text-right">Now</span>
            <span role="columnheader" class="text-right">Peak</span>
            <span role="columnheader" class="text-right">{{ isToday ? 'Share now' : 'Share at peak' }}</span>
          </template>
          <WTableRow
            v-for="r in figs.rows"
            :key="r.band.key"
            :class="['group cursor-pointer py-2.5', 'hov:bg-row-hover press:bg-row-press']"
            :data-band-row="r.band.key"
            @click="toggle(r.band.key)"
            @pointerenter="hovered = r.band.key"
            @pointerleave="hovered = null"
          >
            <span role="cell" class="inline-flex items-center justify-center">
              <button
                type="button"
                role="checkbox"
                :aria-checked="shown.has(r.band.key)"
                :aria-label="`Show ${r.band.label} in the chart`"
                :class="[
                  'inline-flex size-4 cursor-pointer items-center justify-center rounded-sm focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-indicator',
                  shown.has(r.band.key)
                    ? 'bg-primary text-on-primary group-hov:bg-primary-hover group-press:bg-primary-press'
                    : 'border-2 border-control group-hov:bg-chip-off group-press:bg-press',
                ]"
                @focus="hovered = r.band.key"
                @blur="hovered = null"
              >
                <Check v-if="shown.has(r.band.key)" class="size-3" :stroke-width="3" aria-hidden="true" />
              </button>
            </span>
            <div role="cell" class="flex min-w-0 items-center gap-3">
              <span
                aria-hidden="true"
                :class="[
                  'size-3 shrink-0 rounded-xs',
                  hovered === r.band.key ? 'bg-chart-hover' : SWATCH[r.band.colour],
                ]"
              ></span>
              <AccountIcon v-if="r.band.account" :account="r.band.account" />
              <span v-else aria-hidden="true" class="size-6 shrink-0"></span>
              <span class="truncate font-semibold">{{ r.band.label }}</span>
            </div>
            <span v-if="isToday" role="cell" class="text-right tabular-nums">{{ size(r.now ?? 0) }}</span>
            <span role="cell" class="text-right tabular-nums">{{ size(r.peak) }}</span>
            <span role="cell" class="text-right text-secondary tabular-nums">{{
              shareText(isToday ? (r.now ?? 0) : r.peak, total)
            }}</span>
          </WTableRow>
        </WTable>
      </template>
    </template>
  </div>
</template>
