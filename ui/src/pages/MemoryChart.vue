<script setup lang="ts">
// The Memory page's account stack and its optional session slices, drawn by MemoryPlot. The Whole computer scale
// adds the Computer in use line. The pointer and keyboard guide show each account's total, and up to five sessions
// per open group. Arrow keys move a minute, Page Up and Page Down an hour, Home and End to the ends. Escape hides
// the tooltip. The table is the chart's accessible equivalent.
import { useElementSize } from '@vueuse/core';
import { computed, ref, useTemplateRef, watch } from 'vue';
import { CHART, MISSING_TEXT, guideAt, guideText, timeAxis, size, type ChartModel } from '../memory';
import MemoryPlot from './MemoryPlot.vue';

const props = defineProps<{
  model: ChartModel;
  /** The band of the table row under the pointer. */
  hovered: string | null;
  /** The day, so the guide starts again on another one. */
  day: string;
}>();

const box = useTemplateRef<HTMLElement>('box');
const { width: boxWidth } = useElementSize(box);
const width = computed(() => Math.max(0, Math.floor(boxWidth.value) - CHART.gutter));
const span = computed(() => props.model.to - props.model.from);
const x = (m: number) => CHART.gutter + ((m - props.model.from) / span.value) * width.value;

const axis = computed(() => timeAxis(props.model, width.value));

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
const guide = computed(() => (at.value === null ? null : guideAt(props.model, at.value)));
const valueText = computed(() => (guide.value ? guideText(guide.value) : 'No minute chosen'));
/** The guide's place, and which side of it the tooltip opens on so it stays inside the chart. */
const guideX = computed(() => (at.value === null ? null : x(at.value + 0.5)));
const tipLeft = computed(() => guideX.value !== null && guideX.value < boxWidth.value / 2);
const tipRoom = computed(() => {
  const x = guideX.value ?? 0;
  return Math.max(0, (tipLeft.value ? boxWidth.value - x : x) - 12);
});

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
    <MemoryPlot
      :model="props.model"
      :width="width"
      :chart="CHART"
      :axis="axis"
      :hovered="props.hovered"
      :guide-x="guideX"
    />
    <div
      v-if="guide && guideX !== null && !dismissed"
      aria-hidden="true"
      :class="[
        'pointer-events-none absolute top-0 z-10 w-max rounded-md bg-primary px-2.5 py-1.5 text-12 text-on-primary shadow-modal',
        tipLeft ? 'ml-3' : '-ml-3 -translate-x-full',
      ]"
      :style="{
        left: `${guideX}px`,
        maxWidth: `min(${tipRoom}px, var(--spacing) * 64)`,
        minWidth: `min(${tipRoom}px, var(--spacing) * 40)`,
      }"
      data-guide-tip
    >
      <p class="font-semibold">{{ guide.time }}</p>
      <p v-if="guide.missing">{{ MISSING_TEXT[guide.missing] }}</p>
      <template v-else>
        <div v-for="r in guide.rows" :key="r.key">
          <p class="flex justify-between gap-4">
            <span class="truncate">{{ r.label }}</span
            ><span class="shrink-0 tabular-nums">{{ size(r.kib) }}</span>
          </p>
          <p v-for="session in r.sessions" :key="session.key" class="flex justify-between gap-4 pl-3">
            <span class="truncate">{{ session.label }}</span
            ><span class="shrink-0 tabular-nums">{{ size(session.kib) }}</span>
          </p>
          <p v-if="r.more" class="flex justify-between gap-4 pl-3">
            <span>{{ r.more.count }} more</span
            ><span class="shrink-0 tabular-nums">{{ size(r.more.kib) }} in group total</span>
          </p>
        </div>
        <p class="flex justify-between gap-4 font-semibold">
          <span>Total</span><span class="tabular-nums">{{ size(guide.total) }}</span>
        </p>
        <p v-if="guide.computer !== undefined" class="flex justify-between gap-4">
          <span>Computer in use</span><span class="tabular-nums">{{ size(guide.computer) }}</span>
        </p>
      </template>
    </div>
  </div>
</template>
