<script setup lang="ts">
// The Overview's Memory widget links to the Memory page. It follows today's samples while the window is visible.
import { Mountain } from '@lucide/vue';
import { useElementSize } from '@vueuse/core';
import { computed, useTemplateRef } from 'vue';
import WWidget from '../components/WWidget.vue';
import { clock } from '../format';
import { CHART_SMALL, anyRan, bandsOf, chartModel, endAxis, figures, isoDay, memoryReadings, size } from '../memory';
import { useMemoryDay } from '../memoryDay';
import { store, usedAccounts } from '../store';
import MemoryPlot from './MemoryPlot.vue';

const today = computed(() => isoDay(new Date(store.now)));
const { data, error } = useMemoryDay(today);

const all = computed(() =>
  store.state ? [...(store.state.main ? [store.state.main] : []), ...store.state.accounts] : [],
);
const series = computed(() => data.value?.series);
const minutes = computed(() => data.value?.minutes);
const bands = computed(() =>
  series.value ? bandsOf({ series: series.value }, usedAccounts(store.state), all.value) : [],
);
/** Every band: the card leaves none out. */
const every = computed(() => new Set(bands.value.map((b) => b.key)));
const readings = computed(() =>
  minutes.value && series.value ? memoryReadings({ minutes: minutes.value, series: series.value }, bands.value) : null,
);
const figs = computed(() =>
  data.value && readings.value
    ? figures(data.value, bands.value, every.value, true, bands.value, readings.value)
    : null,
);
/** Some AI session ran today: the figures and the chart show, else the line saying none is running. */
const ran = computed(() => !!figs.value && anyRan(figs.value.rows));
const model = computed(() =>
  data.value && readings.value
    ? chartModel(data.value, bands.value, every.value, 2, 1, 'sessions', readings.value)
    : null,
);

const box = useTemplateRef<HTMLElement>('box');
const { width: boxWidth } = useElementSize(box);
const width = computed(() => Math.max(0, Math.floor(boxWidth.value) - CHART_SMALL.gutter));
const axis = computed(() => (model.value ? endAxis(model.value, width.value) : null));

const now = computed(() => size(figs.value?.now ?? 0));
const peak = computed(() =>
  figs.value?.peak && figs.value.peakAt !== null ? { size: size(figs.value.peak), at: clock(figs.value.peakAt) } : null,
);
const message = computed(() => {
  if (error.value && !data.value) return store.connected ? error.value : "Warden's core is not running.";
  if (!data.value) return 'Reading the day…';
  return ran.value ? null : 'No AI sessions are running.';
});
const label = computed(() => {
  const title = "AI sessions' memory today";
  if (message.value) return `${title}: ${message.value}`;
  const current = `${title}: now ${now.value}`;
  return peak.value ? `${current}, peak ${peak.value.size} at ${peak.value.at}` : current;
});
</script>

<template>
  <WWidget title="Memory" href="#/memory" :label="label" data-memory-card>
    <template #body>
      <div class="flex flex-col gap-3">
        <p v-if="message" class="text-13 text-tertiary" :data-card-none="data && !ran ? '' : undefined">
          {{ message }}
        </p>
        <template v-else-if="figs">
          <p class="flex flex-wrap items-baseline gap-x-2 gap-y-1 tabular-nums" data-card-figure="now">
            <strong class="text-24 font-semibold">{{ now }}</strong>
            <span v-if="figs.nowShare && data?.summary.computerTotal" class="text-13 text-secondary">
              {{ figs.nowShare }} of {{ size(data.summary.computerTotal) }}
            </span>
          </p>
          <div ref="box" data-memory-card-chart>
            <MemoryPlot
              v-if="model && axis"
              :model="model"
              :width="width"
              :chart="CHART_SMALL"
              :axis="axis"
              :hovered="null"
              :guide-x="null"
            />
          </div>
        </template>
      </div>
    </template>
    <template v-if="ran && peak" #footer>
      <p class="flex items-center gap-1.5 text-12 text-tertiary tabular-nums" data-card-figure="peak">
        <Mountain aria-hidden="true" class="size-3.5 text-icon" />
        <span>{{ peak.size }} peak at {{ peak.at }}</span>
      </p>
    </template>
  </WWidget>
</template>
