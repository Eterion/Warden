<script setup lang="ts">
// The Memory page: one day at a time, its own address (#/memory is today, #/memory/<yyyy-MM-dd> a past day), so back
// and forward go through the days seen. Previous and Next go to the nearest day with recordings (Next stops at today,
// saying why); between them the day opens a calendar to pick any day (MemoryDayPicker). The figures (MemoryFigures:
// the AI sessions now and their share of the computer, today only; the peak and its time; the computer's memory in use
// with its meter, amber above 90%; Warden itself), then the stack and account table. The Computer switch shows
// whole-computer memory. Open group rows divide their band. Group checks are saved, and session checks stay here.
// Today's page follows each new sample through the long poll (useMemoryDay, which
// the Overview's Memory card shares), paused while the window is hidden; a past day is read once.
import { Check, ChevronDown, ChevronLeft, ChevronRight, Minus } from '@lucide/vue';
import { useResizeObserver } from '@vueuse/core';
import { computed, onMounted, ref, shallowRef, useTemplateRef, watch } from 'vue';
import type { Provider } from '../api';
import OwlIcon from '../components/OwlIcon.vue';
import ProviderMark from '../components/ProviderMark.vue';
import PageHeader from '../components/PageHeader.vue';
import SessionTreeLines from '../components/SessionTreeLines.vue';
import WButton from '../components/WButton.vue';
import WIconButton from '../components/WIconButton.vue';
import WTable from '../components/WTable.vue';
import WTableRow from '../components/WTableRow.vue';
import WWidget from '../components/WWidget.vue';
import {
  bandRows,
  chartModel,
  figures,
  isoDay,
  memoryReadings,
  shareText,
  size,
  stepDays,
  type Band,
  type BandColour,
  type BandRow,
} from '../memory';
import { useMemoryDay } from '../memoryDay';
import {
  OTHER_SIGN_IN,
  groupIsShown,
  memoryCatalog,
  memoryGroups,
  sessionBands,
  showsRecordedPeak,
  sliceKey,
  type SessionBand,
} from '../memoryGroups';
import { memorySettings, memorySettingsError, saveMemoryFilter, saveMemoryScale } from '../memorySettings';
import { go, route } from '../route';
import { store, usedAccounts } from '../store';
import MemoryChart from './MemoryChart.vue';
import MemoryCellText from './MemoryCellText.vue';
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

/**
 * The day shown, then (today, while the window is in sight) each new sample; ended when the page is left. The days
 * with recordings stay while another day loads, so Previous and Next stay as they were.
 */
const { data, days, error } = useMemoryDay(day);

// ---- What is shown ----

const all = computed(() =>
  store.state ? [...(store.state.main ? [store.state.main] : []), ...store.state.accounts] : [],
);
const scale = computed(() => memorySettings.value.memoryScale);
const filter = computed(() => memorySettings.value.memoryFilter);
const series = computed(() => data.value?.series);
const minutes = computed(() => data.value?.minutes);
const catalog = computed(() =>
  series.value ? memoryCatalog(series.value, usedAccounts(store.state), all.value) : null,
);
const bands = computed(() => (catalog.value ? memoryGroups(catalog.value, filter.value.hiddenAccounts) : []));
/** Disclosure and individual session choices stay on the page, across day changes. */
const expanded = ref(new Set<string>());
const sessionChoices = ref(new Map<string, Map<string, boolean>>());
const savingGroups = ref(new Map<string, boolean>());
const openBands = computed(() => {
  const index = catalog.value;
  return new Map(
    index
      ? bands.value
          .filter((group) => expanded.value.has(group.key))
          .map((group) => [group.key, sessionBands(index, group, day.value)] as const)
      : [],
  );
});
type Checked = boolean | 'mixed';
const selection = computed(() =>
  bands.value.map((group) => {
    const show = groupIsShown(group, filter.value);
    const choices = sessionChoices.value.get(group.key);
    const index = catalog.value;
    const included =
      choices?.size && index
        ? group.series.filter((id) => choices.get(sliceKey(group, index.byId.get(id)!, day.value)) ?? show)
        : show
          ? group.series
          : [];
    const checked: Checked =
      group.series.length === 0
        ? show
        : !included.length
          ? false
          : included.length === group.series.length
            ? true
            : 'mixed';
    return { group, included, checked };
  }),
);
const visible = computed<Band[]>(() =>
  selection.value.flatMap(({ group, included, checked }) =>
    checked === false
      ? []
      : [
          {
            ...group,
            series: included,
            slices: openBands.value.get(group.key)?.filter((s) => included.includes(s.series[0]!)),
          },
        ],
  ),
);
const shown = computed(() => new Set(visible.value.map((b) => b.key)));
const readings = computed(() => {
  const samples = minutes.value;
  const info = series.value;
  return samples && info
    ? memoryReadings({ minutes: samples, series: info }, bands.value, visible.value, openBands.value)
    : null;
});
const details = computed(() => {
  const byGroup = new Map<string, BandRow<SessionBand>[]>();
  const open = openBands.value;
  if (!open.size || !data.value || !readings.value) return byGroup;
  for (const [key, slices] of open) {
    byGroup.set(
      key,
      bandRows(data.value, slices, readings.value).sort((a, b) => b.peak - a.peak),
    );
  }
  return byGroup;
});
const figs = computed(() =>
  data.value && readings.value
    ? figures(
        data.value,
        bands.value,
        shown.value,
        showsRecordedPeak(data.value, visible.value),
        visible.value,
        readings.value,
      )
    : null,
);
const model = computed(() =>
  data.value && readings.value
    ? chartModel(
        data.value,
        visible.value.filter((band) => readings.value!.shown.get(band.key)!.peak > 0),
        shown.value,
        3,
        60,
        scale.value,
        readings.value,
      )
    : null,
);
const legend = computed(
  () =>
    figs.value?.rows.filter(({ band, peak }) => peak > 0 && (band.kind !== 'warden' || shown.value.has(band.key))) ??
    [],
);
const allHidden = computed(
  () => !!series.value?.some((s) => s.kind !== 'warden') && !visible.value.some((band) => band.series.length),
);
interface TableRow extends BandRow {
  group: Band;
  checked: Checked;
  session: boolean;
  last: boolean;
  provider?: Provider | null;
  from?: string | null;
}
const rows = computed<TableRow[]>(() =>
  selection.value.flatMap(({ group, included, checked }, i) => {
    const row = figs.value?.rows[i];
    if (!row) return [];
    const parent = {
      ...row,
      group,
      checked: savingGroups.value.get(group.key) ?? checked,
      session: false,
      last: false,
      provider: group.account?.provider,
    };
    const sessions = details.value.get(group.key);
    if (!sessions?.length) return [parent];
    const shownSeries = new Set(included);
    return [
      parent,
      ...sessions.map((detail, index) => ({
        ...detail,
        group,
        checked: shownSeries.has(detail.band.series[0]!),
        session: true,
        last: index === sessions.length - 1,
        provider: group.kind === 'account' ? undefined : detail.band.provider,
        from: detail.band.from,
      })),
    ];
  }),
);
const steps = computed(() => stepDays(days.value, day.value, today.value));
const nothing = computed(() => !!data.value && data.value.minutes.length === 0 && !isToday.value);
/** Today, nothing ran all day: the message can take the chart's place. */
const noSessions = computed(
  () => !!data.value && isToday.value && !figs.value?.rows.some((r) => r.band.kind !== 'warden' && r.peak),
);
/** Today, nothing runs now (some did earlier): the message shows above the day's chart. */
const noneNow = computed(
  () =>
    !!data.value &&
    isToday.value &&
    !noSessions.value &&
    !figs.value?.rows.some((r) => r.band.kind !== 'warden' && r.now),
);
const total = computed(() => data.value?.summary.computerTotal ?? null);
const maxNow = computed(() => figs.value?.rows.reduce((max, r) => Math.max(max, r.now ?? 0), 0) ?? 0);
const hovered = ref<string | null>(null);
const focused = ref<string | null>(null);
const highlighted = computed(() => hovered.value ?? focused.value);
const SWATCH: Record<BandColour, string> = {
  'claude-1': 'bg-chart-claude-1',
  'claude-2': 'bg-chart-claude-2',
  'claude-3': 'bg-chart-claude-3',
  'codex-1': 'bg-chart-codex-1',
  'codex-2': 'bg-chart-codex-2',
  'codex-3': 'bg-chart-codex-3',
  'other-sign-in': 'bg-chart-other-sign-in',
  left: 'bg-chart-left',
  warden: 'bg-chart-warden',
};

function toggleDisclosure(key: string): void {
  if (!expanded.value.delete(key)) expanded.value.add(key);
}

function rowKey(target: EventTarget | null): string | null {
  return target instanceof Element ? (target.closest('[data-band-row]')?.getAttribute('data-band-row') ?? null) : null;
}

// Measure on layout or row changes. Fully visible names do not need a tooltip instance or a focus stop.
const table = useTemplateRef<InstanceType<typeof WTable>>('table');
const clipped = shallowRef(new Set<string>());
function measureText(): void {
  const root = table.value?.$el as HTMLElement | undefined;
  if (!root) return;
  const next = new Set<string>();
  for (const label of root.querySelectorAll<HTMLElement>('[data-memory-name], [data-started-from]')) {
    if (label.scrollWidth <= label.clientWidth) continue;
    const key = rowKey(label);
    if (key) next.add(label.hasAttribute('data-started-from') ? `from:${key}` : key);
  }
  if (next.size !== clipped.value.size || [...next].some((key) => !clipped.value.has(key))) clipped.value = next;
}
useResizeObserver(() => table.value?.$el, measureText);
watch([bands, openBands], measureText, { flush: 'post' });
onMounted(() => {
  void document.fonts?.ready.then(measureText);
});

function onTableClick(event: MouseEvent): void {
  if (!(event.target instanceof Element)) return;
  const key = rowKey(event.target);
  if (!key) return;
  if (event.target.closest('button[aria-expanded]')) toggleDisclosure(key);
  else {
    const row = rows.value.find((r) => r.band.key === key);
    if (row) void toggle(row);
  }
}

function onTableFocus(event: FocusEvent): void {
  focused.value =
    event.target instanceof Element && event.target.matches('[role="checkbox"]') ? rowKey(event.target) : null;
}

async function toggle(r: TableRow): Promise<void> {
  const { key } = r.group;
  const on = r.checked !== true;
  if (r.session) {
    const choices = new Map(sessionChoices.value.get(key));
    choices.set(r.band.key, on);
    sessionChoices.value.set(key, choices);
    return;
  }
  if (savingGroups.value.has(key)) return;
  if (groupIsShown(r.group, filter.value) === on) {
    sessionChoices.value.delete(key);
    return;
  }

  const before = sessionChoices.value.get(key);
  savingGroups.value.set(key, on);
  const saved = await saveMemoryFilter((current) => {
    if (r.group.kind === 'warden') return { ...current, showWarden: on };
    if (r.group.kind === 'left') return { ...current, showLeft: on };
    const id = r.group.account?.id ?? OTHER_SIGN_IN;
    const hiddenAccounts = on
      ? current.hiddenAccounts.filter((hidden) => hidden !== id)
      : [...current.hiddenAccounts, id];
    return { ...current, hiddenAccounts };
  });
  if (saved) {
    // Reset the group's choices, keeping any changes made to individual sessions during the save.
    const newer = new Map(sessionChoices.value.get(key));
    for (const [session, value] of before ?? []) if (newer.get(session) === value) newer.delete(session);
    if (newer.size) sessionChoices.value.set(key, newer);
    else sessionChoices.value.delete(key);
  }
  savingGroups.value.delete(key);
}
</script>

<template>
  <div class="flex flex-col">
    <PageHeader>
      <h1 class="text-20 font-semibold">Memory</h1>
      <div class="ml-auto flex items-center gap-1">
        <h2><MemoryDayPicker :day="day" :today="today" :recorded="days" @pick="show" /></h2>
        <WIconButton
          label="Previous day"
          :icon="ChevronLeft"
          size="sm"
          :disabled-reason="steps.prev ? null : 'No earlier day was recorded.'"
          @click="steps.prev && show(steps.prev)"
        />
        <WIconButton
          label="Next day"
          :icon="ChevronRight"
          size="sm"
          :disabled-reason="steps.next ? null : 'This is today: there is no later day.'"
          @click="steps.next && show(steps.next)"
        />
        <WButton
          variant="quiet"
          size="sm"
          on="panel"
          class="ml-1"
          :disabled-reason="isToday ? 'This is today.' : null"
          @click="show(today)"
          >Today</WButton
        >
      </div>
    </PageHeader>

    <p v-if="error && !data" class="text-14 text-secondary">
      {{ store.connected ? error : "Warden's core is not running. Start it, and this page fills in by itself." }}
    </p>
    <p v-else-if="!data" class="text-14 text-secondary">Reading the day…</p>
    <div
      v-else-if="nothing"
      data-memory-empty
      class="rounded-lg border border-divider-strong p-4 text-14 text-tertiary"
    >
      Nothing recorded on this day.
    </div>
    <div v-else-if="figs" class="flex flex-col gap-4">
      <MemoryFigures
        :figures="figs"
        :today="isToday"
        :total="total"
        :model="model"
        :show-computer="scale === 'computer'"
        @update:show-computer="saveMemoryScale($event ? 'computer' : 'sessions')"
      />

      <div
        v-if="noSessions && !allHidden && !model?.computer && (figs.warden === null || !shown.has('warden'))"
        class="flex flex-col items-start rounded-lg border border-divider-strong p-4 text-14 text-tertiary"
        data-memory-none
      >
        No AI sessions are running.
      </div>
      <WWidget v-else title="Memory through the day">
        <template #header>
          <ul
            v-if="legend.length || model?.computer"
            class="flex min-w-0 flex-wrap gap-x-3.5 gap-y-1 text-12 text-secondary"
            aria-label="Chart legend"
          >
            <li
              v-for="{ band } in legend"
              :key="band.key"
              :class="['inline-flex min-w-0 items-center gap-1.5', !shown.has(band.key) && 'text-tertiary']"
              :data-legend-band="band.key"
            >
              <span
                aria-hidden="true"
                :class="[
                  'size-2.5 shrink-0 rounded-xs',
                  shown.has(band.key) ? SWATCH[band.colour] : 'border border-control',
                ]"
              ></span>
              <span class="wrap-anywhere"
                >{{ band.label }}<span v-if="!shown.has(band.key)" class="sr-only"> (hidden)</span></span
              >
            </li>
            <li v-if="model?.computer" class="inline-flex items-center gap-1.5">
              <span aria-hidden="true" class="w-3.5 border-t border-text"></span>Computer in use
            </li>
          </ul>
        </template>
        <template #body>
          <div class="flex flex-col gap-4">
            <p v-if="allHidden && !model?.computer" class="text-14 text-tertiary" data-memory-all-hidden>
              Nothing is ticked in the table.
            </p>
            <p v-if="!allHidden && (noneNow || noSessions)" class="text-14 text-tertiary" data-memory-none-now>
              No AI sessions are running.
            </p>
            <MemoryChart
              v-if="model && (!allHidden || model.computer)"
              :model="model"
              :hovered="highlighted"
              :day="day"
            />
          </div>
        </template>
      </WWidget>

      <p v-if="memorySettingsError" role="status" class="text-13 text-attn">{{ memorySettingsError }}</p>
      <WTable
        ref="table"
        on="panel"
        aria-label="Memory by account"
        :cols="isToday ? 'grid-cols-memory gap-x-2' : 'grid-cols-memory-past gap-x-2'"
        class="min-w-memory"
        data-memory-table
        @click="onTableClick"
        @pointerover="hovered = rowKey($event.target)"
        @pointerout="hovered = rowKey($event.relatedTarget)"
        @focusin="onTableFocus"
        @focusout="focused = null"
      >
        <template #head>
          <span role="columnheader"><span class="sr-only">Shown in the chart</span></span>
          <span role="columnheader">Account</span>
          <span role="columnheader" class="whitespace-nowrap">Started from</span>
          <span v-if="isToday" role="columnheader" class="text-right">Now</span>
          <span role="columnheader" class="text-right">Peak</span>
          <span role="columnheader" class="text-right whitespace-nowrap">{{
            isToday ? 'Share now' : 'Share at peak'
          }}</span>
        </template>
        <WTableRow
          v-for="r in rows"
          :key="r.band.key"
          class="group cursor-pointer py-2.25 text-13 hov:bg-row-hover press:not-has-[[aria-expanded]:active]:not-has-[[data-started-from]:active]:bg-row-press"
          :data-band-row="r.band.key"
          :data-session-of="r.session ? r.group.key : undefined"
        >
          <span role="cell" class="inline-flex items-center justify-center">
            <button
              type="button"
              role="checkbox"
              :aria-checked="r.checked"
              :aria-disabled="(!r.session && savingGroups.has(r.group.key)) || undefined"
              :aria-label="`Show ${r.band.label}${r.session ? ` under ${r.group.label}` : ''} in the chart`"
              :class="[
                'inline-flex size-4 cursor-pointer items-center justify-center rounded-sm focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-indicator',
                r.checked !== false
                  ? 'bg-primary text-on-primary group-hov:bg-primary-hover group-not-has-[[aria-expanded]:active]:group-not-has-[[data-started-from]:active]:group-press:bg-primary-press'
                  : 'border-2 border-control group-hov:bg-chip-off group-not-has-[[aria-expanded]:active]:group-not-has-[[data-started-from]:active]:group-press:bg-press',
              ]"
            >
              <Check v-if="r.checked === true" class="size-3" :stroke-width="3" aria-hidden="true" />
              <Minus v-else-if="r.checked === 'mixed'" class="size-3" :stroke-width="3" aria-hidden="true" />
            </button>
          </span>
          <div role="cell" class="flex min-w-0 items-center self-stretch">
            <SessionTreeLines v-if="r.session" :depth="1" :rails="[]" :last="r.last" disclosure />
            <div v-else class="size-6 shrink-0">
              <WIconButton
                v-if="r.group.kind !== 'warden' && r.group.series.length"
                size="xs"
                :icon="expanded.has(r.group.key) ? ChevronDown : ChevronRight"
                :label="`${expanded.has(r.group.key) ? 'Hide' : 'Show'} the sessions of ${r.group.label}`"
                :aria-expanded="expanded.has(r.group.key)"
              />
            </div>
            <span
              aria-hidden="true"
              data-memory-swatch
              :class="[
                'mr-2 size-3 shrink-0 rounded-xs',
                highlighted === r.band.key ? 'bg-chart-hover' : SWATCH[r.band.colour],
              ]"
            ></span>
            <span class="flex min-w-0 items-center gap-1.5">
              <OwlIcon v-if="r.band.kind === 'warden'" :size="12" class="shrink-0 text-icon" aria-hidden="true" />
              <ProviderMark
                v-else-if="r.provider"
                :provider="r.provider"
                size="xs"
                :label="r.provider === 'claude' ? 'Claude' : 'Codex'"
              />
              <MemoryCellText
                :text="r.band.label"
                :clipped="clipped.has(r.band.key)"
                class="font-medium"
                data-memory-name
              />
            </span>
          </div>
          <div role="cell" class="min-w-0">
            <MemoryCellText
              v-if="r.from"
              :text="r.from"
              :clipped="clipped.has(`from:${r.band.key}`)"
              class="text-12 text-tertiary"
              data-started-from
            />
          </div>
          <span v-if="isToday" role="cell" class="flex items-center justify-end gap-1.5 text-secondary tabular-nums">
            <span aria-hidden="true" class="relative h-2 w-8 shrink-0 rounded-full bg-track" data-now-track>
              <!-- The panel-colour edge separates the fill from the grey track in every row state. -->
              <span v-if="r.now && maxNow" class="absolute inset-px">
                <span
                  :class="['block h-full rounded-full ring-1 ring-panel', SWATCH[r.band.colour]]"
                  :style="{ width: `${(r.now / maxNow) * 100}%` }"
                  data-now-meter
                ></span>
              </span>
            </span>
            <span class="w-14 shrink-0 text-right whitespace-nowrap">{{ size(r.now ?? 0) }}</span>
          </span>
          <span role="cell" class="text-right whitespace-nowrap text-secondary tabular-nums">{{ size(r.peak) }}</span>
          <span role="cell" class="text-right whitespace-nowrap text-tertiary tabular-nums">{{
            shareText(isToday ? (r.now ?? 0) : r.peak, total)
          }}</span>
        </WTableRow>
      </WTable>
    </div>
  </div>
</template>
