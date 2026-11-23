<script setup lang="ts">
// The Memory page's chart, drawn by hand in SVG (no chart library): the minutes across the day's recorded span, memory
// up with three faint gridlines labelled in GB, a stacked band per account (greys, the band of the hovered table row
// darkest), each divided from the next by a thin line in the panel's colour, and the minutes Warden was not running as
// an empty band. A guide follows the pointer with a tooltip of the shown bands and their total at that minute; the chart
// is a slider for the keyboard (the arrow keys move the guide a minute, Page Up and Page Down an hour, Home and End to
// the first and last recording), its value the same words, so it works without a mouse; Escape hides the tooltip. The table under it is its accessible equivalent.
import { useElementSize } from '@vueuse/core';
import { computed, ref, useTemplateRef, watch } from 'vue';
import { CHART, MISSING_TEXT, bandPaths, guideAt, guideText, hourTicks, size, type ChartModel } from '../memory';

const props = defineProps<{
  model: ChartModel;
  /** The bands that ran something that day: the guide lists only those. */
  ran: Set<string>;
  /** The band of the table row under the pointer. */
  hovered: string | null;
  /** The day, so the guide starts again on another one. */
  day: string;
}>();

const TONE = ['', 'fill-chart-1', 'fill-chart-2', 'fill-chart-3', 'fill-chart-4', 'fill-chart-5'];

const box = useTemplateRef<HTMLElement>('box');
const { width: boxWidth } = useElementSize(box);
const width = computed(() => Math.max(0, Math.floor(boxWidth.value) - CHART.gutter));
const span = computed(() => props.model.to - props.model.from);
const x = (m: number) => CHART.gutter + ((m - props.model.from) / span.value) * width.value;
const y = (kib: number) => CHART.top + (1 - kib / props.model.top) * (CHART.height - CHART.top);

const paths = computed(() => bandPaths(props.model, width.value));
const ticks = computed(() => hourTicks(props.model, width.value));
const gaps = computed(() =>
  props.model.gaps.map((g) => ({ ...g, x: x(g.from), w: x(g.to) - x(g.from) })).filter((g) => g.w > 0),
);

/** The guide's minute, any minute of the span: in a gap the guide says Warden was not running. */
const at = ref<number | null>(null);
const focused = ref(false);
/** Escape hides the tooltip (the guide stays) until the guide moves again. */
const dismissed = ref(false);
watch(
  () => props.day,
  () => (at.value = null),
);
watch(at, () => (dismissed.value = false));

const first = computed(() => props.model.minutes[0]?.m ?? props.model.from);
const last = computed(() => props.model.minutes[props.model.minutes.length - 1]?.m ?? props.model.to - 1);
const guide = computed(() => (at.value === null ? null : guideAt(props.model, at.value, props.ran)));
const valueText = computed(() => (guide.value ? guideText(guide.value) : 'No minute chosen'));
/** The guide's place, and which side of it the tooltip opens on so it stays inside the chart. */
const guideX = computed(() => (at.value === null ? null : x(at.value + 0.5)));
const tipLeft = computed(() => guideX.value !== null && guideX.value < CHART.gutter + width.value / 2);

const clamp = (m: number) => Math.max(props.model.from, Math.min(props.model.to - 1, m));

function onPointer(e: PointerEvent): void {
  const rect = box.value?.getBoundingClientRect();
  if (!rect || width.value <= 0) return;
  const px = e.clientX - rect.left - CHART.gutter;
  if (px < 0 || px > width.value) return;
  at.value = clamp(Math.floor(props.model.from + (px / width.value) * span.value));
}

function onLeave(): void {
  if (!focused.value) at.value = null;
}

function onFocus(): void {
  focused.value = true;
  if (at.value === null) at.value = last.value;
}

function onBlur(): void {
  focused.value = false;
  at.value = null;
}

function onKey(e: KeyboardEvent): void {
  if (e.key === 'Escape') {
    if (at.value !== null && !dismissed.value) {
      dismissed.value = true;
      e.preventDefault();
    }
    return;
  }
  const current = at.value ?? last.value;
  const next: Record<string, () => number> = {
    ArrowLeft: () => current - 1,
    ArrowDown: () => current - 1,
    ArrowRight: () => current + 1,
    ArrowUp: () => current + 1,
    PageUp: () => current + 60,
    PageDown: () => current - 60,
    Home: () => first.value,
    End: () => last.value,
  };
  const go = next[e.key];
  if (!go) return;
  e.preventDefault();
  at.value = clamp(go());
}
</script>

<template>
  <div
    ref="box"
    role="slider"
    tabindex="0"
    aria-label="Memory over the day"
    aria-orientation="horizontal"
    :aria-valuemin="props.model.from"
    :aria-valuemax="props.model.to - 1"
    :aria-valuenow="at ?? last"
    :aria-valuetext="valueText"
    class="relative rounded-lg outline-none focus-visible:outline-2 focus-visible:outline-offset-4 focus-visible:outline-indicator"
    data-memory-chart
    @pointermove="onPointer"
    @pointerleave="onLeave"
    @focus="onFocus"
    @blur="onBlur"
    @keydown="onKey"
  >
    <svg
      :width="CHART.gutter + width"
      :height="CHART.height + CHART.axis"
      class="block overflow-visible text-12"
      aria-hidden="true"
    >
      <!-- Hours Warden was not running: an empty band, named where it fits. -->
      <g v-for="g in gaps" :key="g.from">
        <rect :x="g.x" :y="CHART.top" :width="g.w" :height="CHART.height - CHART.top" class="fill-chart-gap" />
        <text
          v-if="g.w >= 128"
          :x="g.x + g.w / 2"
          :y="CHART.top + (CHART.height - CHART.top) / 2"
          text-anchor="middle"
          dominant-baseline="middle"
          class="fill-tertiary"
          data-not-running
        >
          Warden not running
        </text>
      </g>
      <!-- Three faint gridlines, labelled in GB, and the baseline. -->
      <g v-for="line in props.model.grid" :key="line.kib">
        <line
          :x1="CHART.gutter"
          :x2="CHART.gutter + width"
          :y1="y(line.kib)"
          :y2="y(line.kib)"
          class="stroke-divider-strong"
          stroke-dasharray="2 3"
        />
        <text :x="CHART.gutter - 8" :y="y(line.kib)" text-anchor="end" dominant-baseline="middle" class="fill-tertiary">
          {{ line.label }}
        </text>
      </g>
      <line
        :x1="CHART.gutter"
        :x2="CHART.gutter + width"
        :y1="CHART.height"
        :y2="CHART.height"
        class="stroke-divider-strong"
      />
      <!-- The bands, bottom first, each edged on top by a thin line in the panel's colour. -->
      <g :transform="`translate(${CHART.gutter} 0)`">
        <path
          v-for="p in paths"
          :key="p.key"
          :d="p.area"
          :class="props.hovered === p.key ? 'fill-chart-hover' : TONE[p.tone]"
          :data-band="p.key"
        />
        <path
          v-for="p in paths"
          :key="`${p.key}-edge`"
          :d="p.edge"
          class="fill-none stroke-panel"
          stroke-width="1"
          stroke-linejoin="round"
        />
      </g>
      <!-- The hours along the bottom. -->
      <text
        v-for="t in ticks"
        :key="t.m"
        :x="CHART.gutter + t.x"
        :y="CHART.height + CHART.axis - 4"
        text-anchor="middle"
        class="fill-tertiary"
      >
        {{ t.label }}
      </text>
      <!-- The guide: a line in the text colour on a wider one in the panel's, so it stands out on every band. -->
      <g v-if="guideX !== null" data-guide>
        <line :x1="guideX" :x2="guideX" :y1="CHART.top" :y2="CHART.height" class="stroke-panel" stroke-width="3" />
        <line :x1="guideX" :x2="guideX" :y1="CHART.top" :y2="CHART.height" class="stroke-text" stroke-width="1" />
      </g>
    </svg>
    <div
      v-if="guide && guideX !== null && !dismissed"
      aria-hidden="true"
      :class="[
        'pointer-events-none absolute top-0 z-10 min-w-40 rounded-md bg-primary px-2.5 py-1.5 text-12 text-on-primary shadow-modal',
        tipLeft ? 'ml-3' : '-ml-3 -translate-x-full',
      ]"
      :style="{ left: `${guideX}px` }"
      data-guide-tip
    >
      <p class="font-semibold">{{ guide.time }}</p>
      <p v-if="guide.missing">{{ MISSING_TEXT[guide.missing] }}</p>
      <template v-else>
        <p v-for="r in guide.rows" :key="r.key" class="flex justify-between gap-4">
          <span class="truncate">{{ r.label }}</span
          ><span class="tabular-nums">{{ size(r.kib) }}</span>
        </p>
        <p class="flex justify-between gap-4 font-semibold">
          <span>Total</span><span class="tabular-nums">{{ size(guide.total) }}</span>
        </p>
      </template>
    </div>
  </div>
</template>
