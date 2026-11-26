<script setup lang="ts">
// The Memory page: one day at a time, its own address (#/memory is today, #/memory/<yyyy-MM-dd> a past day), so back
// and forward go through the days seen. Previous and Next go to the nearest day with recordings (Next stops at today,
// saying why); between them the day opens a calendar to pick any day (MemoryDayPicker). The figures (MemoryFigures:
// the AI sessions now and their share of the computer, today only; the peak and its time; the computer's memory in use
// with its meter, amber above 90%; Warden itself), then the controls, stack and account table. Open account rows show
// their sessions and divide their band. Checkboxes and highlights work on whole bands or individual slices.
// Today's page follows each new sample through the long poll (useMemoryDay, which
// the Overview's Memory card shares), paused while the window is hidden; a past day is read once.
import { Check, ChevronDown, ChevronLeft, ChevronRight, ListFilter, Minus } from '@lucide/vue';
import { DropdownMenuPortal, DropdownMenuRoot, DropdownMenuSeparator, DropdownMenuTrigger } from 'reka-ui';
import { computed, ref } from 'vue';
import type { MemoryFilter, MemoryScale, Provider } from '../api';
import { count } from '../format';
import OwlIcon from '../components/OwlIcon.vue';
import ProviderMark from '../components/ProviderMark.vue';
import PageHeader from '../components/PageHeader.vue';
import SegmentedControl from '../components/SegmentedControl.vue';
import SessionTreeLines from '../components/SessionTreeLines.vue';
import WButton from '../components/WButton.vue';
import WIconButton from '../components/WIconButton.vue';
import WMenuCheckboxItem from '../components/WMenuCheckboxItem.vue';
import WMenuContent from '../components/WMenuContent.vue';
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
  memoryCatalog,
  memoryGroups,
  sessionBands,
  showsRecordedPeak,
  sliceKey,
  type SessionBand,
} from '../memoryGroups';
import { memorySettings, memorySettingsError, saveMemorySettings } from '../memorySettings';
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
const SCALES: { value: MemoryScale; label: string }[] = [
  { value: 'sessions', label: 'AI sessions' },
  { value: 'computer', label: 'Whole computer' },
];
const series = computed(() => data.value?.series);
const minutes = computed(() => data.value?.minutes);
const catalog = computed(() =>
  series.value ? memoryCatalog(series.value, usedAccounts(store.state), all.value) : null,
);
const grouping = computed(() => (catalog.value ? memoryGroups(catalog.value, filter.value) : null));
const bands = computed(() => grouping.value?.bands ?? []);
/** Disclosure and selection stay on the page, across day changes. */
const expanded = ref(new Set<string>());
const hiddenGroups = ref(new Set<string>());
const hiddenSessions = ref(new Map<string, Set<string>>());
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
    const hidden = hiddenGroups.value.has(group.key);
    const omitted = hiddenSessions.value.get(group.key);
    const index = catalog.value;
    const included = hidden
      ? []
      : omitted?.size && index
        ? group.series.filter((id) => !omitted.has(sliceKey(group, index.byId.get(id)!, day.value)))
        : group.series;
    const checked: Checked =
      hidden || (group.series.length > 0 && !included.length)
        ? false
        : included.length < group.series.length
          ? 'mixed'
          : true;
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
    ? chartModel(data.value, visible.value, shown.value, 3, 60, scale.value, readings.value)
    : null,
);
const hasWarden = computed(() => bands.value.some((band) => band.kind === 'warden'));
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
    const parent = { ...row, group, checked, session: false, last: false, provider: group.account?.provider };
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
/** Today, nothing ran all day: the message takes the chart's and the table's place. */
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

function onTableClick(event: MouseEvent): void {
  const key = rowKey(event.target);
  if (!key) return;
  if (event.target instanceof Element && event.target.closest('button[aria-expanded]')) toggleDisclosure(key);
  else {
    const row = rows.value.find((r) => r.band.key === key);
    if (row) toggle(row);
  }
}

function onTableFocus(event: FocusEvent): void {
  focused.value =
    event.target instanceof Element && event.target.matches('[role="checkbox"]') ? rowKey(event.target) : null;
}

function toggle(r: TableRow): void {
  const { key } = r.group;
  if (!r.session) {
    if (r.checked === true) hiddenGroups.value.add(key);
    else {
      hiddenGroups.value.delete(key);
      hiddenSessions.value.delete(key);
    }
    return;
  }
  if (hiddenGroups.value.delete(key)) {
    hiddenSessions.value.set(
      key,
      new Set((openBands.value.get(key) ?? []).map((b) => b.key).filter((id) => id !== r.band.key)),
    );
    return;
  }
  const omitted = new Set(hiddenSessions.value.get(key));
  if (!omitted.delete(r.band.key)) omitted.add(r.band.key);
  hiddenSessions.value.set(key, omitted);
}

const announced = ref('');
const hiddenLabel = computed(() => (grouping.value?.hiddenCount ? `${count(grouping.value.hiddenCount)} hidden` : ''));

function chooseFilter(patch: Partial<MemoryFilter>): void {
  saveMemorySettings({ memoryFilter: { ...filter.value, ...patch } });
  announced.value = grouping.value?.allHidden
    ? 'The filter leaves out everything.'
    : patch.showWarden === undefined
      ? hiddenLabel.value || '0 hidden'
      : patch.showWarden
        ? 'Warden itself shown.'
        : 'Warden itself hidden.';
}

function showAccount(id: string, on: boolean): void {
  const rest = filter.value.hiddenAccounts.filter((hiddenId) => hiddenId !== id);
  chooseFilter({ hiddenAccounts: on ? rest : [...rest, id] });
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
      <MemoryFigures :figures="figs" :today="isToday" :total="total" :model="model" />

      <div class="flex flex-wrap items-center gap-x-4 gap-y-2" data-memory-controls>
        <SegmentedControl
          label="Memory scale"
          :options="SCALES"
          :model-value="scale"
          @update:model-value="saveMemorySettings({ memoryScale: $event })"
        />
        <div class="ml-auto flex flex-wrap items-center justify-end gap-2">
          <span v-if="hiddenLabel" class="text-12 text-secondary tabular-nums" data-memory-hidden>
            {{ hiddenLabel }}
          </span>
          <DropdownMenuRoot :modal="false">
            <DropdownMenuTrigger as-child>
              <WButton
                variant="quiet"
                size="sm"
                on="panel"
                :icon="ListFilter"
                :aria-label="hiddenLabel ? `Filter memory, ${hiddenLabel}` : 'Filter memory'"
                >Filter</WButton
              >
            </DropdownMenuTrigger>
            <DropdownMenuPortal disabled>
              <WMenuContent>
                <WMenuCheckboxItem
                  v-for="choice in grouping?.choices"
                  :key="choice.id"
                  :model-value="!filter.hiddenAccounts.includes(choice.id)"
                  @update:model-value="showAccount(choice.id, $event)"
                >
                  <template #mark>
                    <span class="inline-flex size-4 items-center justify-center" aria-hidden="true">
                      <ProviderMark v-if="choice.band.account" :provider="choice.band.account.provider" size="sm" />
                    </span>
                  </template>
                  {{ choice.band.label }}
                  <template #end>
                    <span aria-hidden="true">{{ count(choice.count) }}</span>
                    <span class="sr-only"
                      >, {{ count(choice.count) }} {{ choice.count === 1 ? 'session' : 'sessions' }}</span
                    >
                  </template>
                </WMenuCheckboxItem>
                <DropdownMenuSeparator v-if="grouping?.choices.length" class="-mx-1 my-1 h-px bg-divider-strong" />
                <WMenuCheckboxItem
                  :model-value="filter.showLeft"
                  @update:model-value="chooseFilter({ showLeft: $event })"
                >
                  <template #mark><span class="size-4" aria-hidden="true"></span></template>
                  Left running
                  <template #end>
                    <span aria-hidden="true">{{ count(grouping?.leftCount ?? 0) }}</span>
                    <span class="sr-only"
                      >, {{ count(grouping?.leftCount ?? 0) }}
                      {{ grouping?.leftCount === 1 ? 'session' : 'sessions' }}</span
                    >
                  </template>
                </WMenuCheckboxItem>
                <WMenuCheckboxItem
                  :model-value="filter.showWarden"
                  @update:model-value="chooseFilter({ showWarden: $event })"
                >
                  <template #mark><OwlIcon :size="16" aria-hidden="true" /></template>
                  Warden itself
                </WMenuCheckboxItem>
              </WMenuContent>
            </DropdownMenuPortal>
          </DropdownMenuRoot>
        </div>
      </div>
      <p role="status" class="sr-only" data-memory-filter-status>{{ announced }}</p>
      <p v-if="memorySettingsError" role="status" class="text-13 text-attn">{{ memorySettingsError }}</p>

      <div
        v-if="grouping?.allHidden"
        class="rounded-lg border border-divider-strong p-4 text-14 text-tertiary"
        data-memory-all-hidden
      >
        The filter leaves out everything.
      </div>
      <div
        v-else-if="noSessions && !model?.computer && !hasWarden"
        class="flex flex-col items-start rounded-lg border border-divider-strong p-4 text-14 text-tertiary"
        data-memory-none
      >
        No AI sessions are running.
      </div>
      <template v-else>
        <WWidget title="By account">
          <template #header>
            <ul
              class="ml-auto flex min-w-0 flex-wrap gap-x-3.5 gap-y-1 text-12 text-secondary"
              aria-label="Chart legend"
            >
              <li
                v-for="band in bands"
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
              <p v-if="noneNow || noSessions" class="text-14 text-tertiary" data-memory-none-now>
                No AI sessions are running.
              </p>
              <MemoryChart v-if="model" :model="model" :hovered="highlighted" :day="day" />
            </div>
          </template>
        </WWidget>

        <WWidget v-if="rows.length && (!noSessions || hasWarden)" title="Accounts">
          <WTable
            bare
            on="panel"
            aria-label="Memory by account"
            cols="grid-cols-memory gap-x-4"
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
              <span v-if="isToday" role="columnheader" class="text-right">Now</span>
              <span role="columnheader" class="text-right">Peak</span>
              <span role="columnheader" class="text-right">{{ isToday ? 'Share now' : 'Share at peak' }}</span>
            </template>
            <WTableRow
              v-for="r in rows"
              :key="r.band.key"
              class="group cursor-pointer py-2.25 text-13 hov:bg-row-hover press:not-has-[[aria-expanded]:active]:bg-row-press"
              :data-band-row="r.band.key"
              :data-session-of="r.session ? r.group.key : undefined"
            >
              <span role="cell" class="inline-flex items-center justify-center">
                <button
                  type="button"
                  role="checkbox"
                  :aria-checked="r.checked"
                  :aria-label="`Show ${r.band.label}${r.session ? ` under ${r.group.label}` : ''} in the chart`"
                  :class="[
                    'inline-flex size-4 cursor-pointer items-center justify-center rounded-sm focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-indicator',
                    r.checked !== false
                      ? 'bg-primary text-on-primary group-hov:bg-primary-hover group-not-has-[[aria-expanded]:active]:group-press:bg-primary-press'
                      : 'border-2 border-control group-hov:bg-chip-off group-not-has-[[aria-expanded]:active]:group-press:bg-press',
                  ]"
                >
                  <Check v-if="r.checked === true" class="size-3" :stroke-width="3" aria-hidden="true" />
                  <Minus v-else-if="r.checked === 'mixed'" class="size-3" :stroke-width="3" aria-hidden="true" />
                </button>
              </span>
              <div role="cell" class="flex min-w-0 items-center self-stretch">
                <SessionTreeLines v-if="r.session" :depth="1" :rails="[]" :last="r.last" disclosure />
                <div v-else class="w-6 shrink-0">
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
                  <span class="flex min-w-0 flex-col">
                    <span class="truncate font-medium" :title="r.band.label">{{ r.band.label }}</span>
                    <span v-if="r.from" class="truncate text-12 text-tertiary" :title="`from ${r.from}`"
                      >from {{ r.from }}</span
                    >
                  </span>
                </span>
              </div>
              <span
                v-if="isToday"
                role="cell"
                class="flex items-center justify-end gap-1.5 text-secondary tabular-nums"
              >
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
              <span role="cell" class="text-right text-secondary tabular-nums">{{ size(r.peak) }}</span>
              <span role="cell" class="text-right text-tertiary tabular-nums">{{
                shareText(isToday ? (r.now ?? 0) : r.peak, total)
              }}</span>
            </WTableRow>
          </WTable>
        </WWidget>
      </template>
    </div>
  </div>
</template>
