<script setup lang="ts">
// The Memory page: one day at a time, its own address (#/memory is today, #/memory/<yyyy-MM-dd> a past day), so back
// and forward go through the days seen. Previous and Next go to the nearest day with recordings (Next stops at today,
// saying why). The figures (now and its share of the computer, today only; the peak and its time; the computer's
// memory in use, amber above 90%; Warden itself), then the chart of the AI sessions' memory stacked by account, and the
// table: the chart's legend and accessible equivalent, a checkbox per row leaving its band out (on this page only),
// hovering a row darkening its band. Today's page follows each new sample through its own long poll, paused while the
// window is hidden; a past day is read once.
import { Check, ChevronLeft, ChevronRight } from 'lucide-vue-next';
import { computed, ref, shallowRef, watch } from 'vue';
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
  dayTitle,
  figures,
  isoDay,
  mergeToday,
  shareText,
  size,
  stepDays,
} from '../memory';
import { clock } from '../format';
import { go, route } from '../route';
import { store, usedAccounts } from '../store';
import MemoryChart from './MemoryChart.vue';

const today = computed(() => isoDay(new Date(store.now)));
/** The day shown: the address's, never one to come. */
const day = computed(() => {
  const d = route.day.value;
  return d && d < today.value ? d : today.value;
});
const isToday = computed(() => day.value === today.value);
const title = computed(() => dayTitle(day.value, today.value));

function show(d: string): void {
  go(d === today.value ? 'memory' : `memory/${d}`);
}

// ---- The day, from the server ----

const data = shallowRef<MemoryDay | null>(null);
const error = ref<string | null>(null);
let controller: AbortController | null = null;
const RETRY_MS = 5000;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

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
      data.value = await api.memoryDay(d, mine.signal);
      error.value = null;
      break;
    } catch (e) {
      if (mine.signal.aborted) return;
      error.value = e instanceof ApiError ? e.message : 'Warden could not be reached.';
      if (e instanceof ApiError) return;
      await sleep(RETRY_MS);
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
      error.value = null;
    } catch (e) {
      if (mine.signal.aborted) return;
      if (!(e instanceof ApiError)) error.value = 'Warden could not be reached.';
      await sleep(RETRY_MS);
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
const steps = computed(() => stepDays(data.value?.days ?? [], day.value, today.value));
const nothing = computed(() => !!data.value && data.value.minutes.length === 0 && !isToday.value);
const noSessions = computed(() => !!data.value && isToday.value && !anyRan(figs.value?.rows ?? []));
const total = computed(() => data.value?.summary.computerTotal ?? null);
const hovered = ref<string | null>(null);
const SWATCH = ['', 'bg-chart-1', 'bg-chart-2', 'bg-chart-3', 'bg-chart-4', 'bg-chart-5'];

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
          :disabled-reason="steps.prev ? undefined : 'No earlier day was recorded.'"
          @click="steps.prev && show(steps.prev)"
        />
        <h2 class="min-w-52 text-center text-16 font-semibold" data-day>{{ title }}</h2>
        <WIconButton
          label="Next day"
          :icon="ChevronRight"
          :disabled-reason="steps.next ? undefined : 'This is today: there is no later day.'"
          @click="steps.next && show(steps.next)"
        />
        <WButton variant="quiet" :disabled-reason="isToday ? 'This is today.' : undefined" @click="show(today)"
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
      <!-- The figures, plain text in a row. -->
      <dl class="flex flex-wrap gap-x-10 gap-y-3" data-figures>
        <div v-if="isToday" class="flex flex-col">
          <dt class="text-12 text-secondary">Now</dt>
          <dd class="text-16 font-semibold tabular-nums">{{ size(figs.now ?? 0) }}</dd>
          <dd v-if="total" class="text-12 text-secondary tabular-nums">{{ figs.nowShare }} of {{ size(total) }}</dd>
        </div>
        <div class="flex flex-col">
          <dt class="text-12 text-secondary">Peak</dt>
          <dd class="text-16 font-semibold tabular-nums">{{ size(figs.peak ?? 0) }}</dd>
          <dd v-if="figs.peakAt !== null && figs.peak" class="text-12 text-secondary tabular-nums">
            at {{ clock(figs.peakAt) }}
          </dd>
        </div>
        <div v-if="figs.computer" class="flex flex-col" data-computer>
          <dt class="text-12 text-secondary">{{ isToday ? 'Computer' : 'Computer at its highest' }}</dt>
          <dd :class="['text-16 font-semibold tabular-nums', figs.computer.high && 'text-attn']">
            {{ size(figs.computer.used) }} of {{ size(figs.computer.total) }}
          </dd>
          <dd :class="['text-12 tabular-nums', figs.computer.high ? 'font-medium text-attn' : 'text-secondary']">
            {{ shareText(figs.computer.used, figs.computer.total) }} in use
          </dd>
        </div>
        <div v-if="figs.warden !== null" class="flex flex-col">
          <dt class="text-12 text-secondary">{{ isToday ? 'Warden itself' : 'Warden at its highest' }}</dt>
          <dd class="text-16 font-semibold tabular-nums">{{ size(figs.warden) }}</dd>
        </div>
      </dl>

      <div
        v-if="noSessions"
        class="flex flex-col items-start rounded-lg border border-divider-strong p-4 text-14 text-tertiary"
        data-memory-none
      >
        No AI sessions are running.
      </div>
      <template v-else>
        <section aria-label="Memory over the day" class="rounded-lg border border-divider-strong px-4 pt-4 pb-2">
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
                :class="['size-3 shrink-0 rounded-xs', hovered === r.band.key ? 'bg-chart-hover' : SWATCH[r.band.tone]]"
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
