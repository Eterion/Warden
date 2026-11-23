<script setup lang="ts">
// The Overview's Memory card, above the limit resets: today's AI sessions' memory at a glance. Its heading is a link to
// the Memory page, with today's figures beside it in the legends' type (the AI sessions now with their share of the
// computer, and the day's peak with its time); under it the Memory page's chart made small (MemoryPlot: the same bands,
// colours and gaps, two gridlines, the times only at its ends), one link to the Memory page named by what it shows (the
// minute-by-minute guide stays on that page). Nothing ran today: a line says so in its place. Today follows each sample
// through the same wait as the Memory page (useMemoryDay), paused while the window is hidden, ended when the Overview is
// left.
import { ChevronRight, CpuAi, Mountain } from '@lucide/vue';
import { useElementSize } from '@vueuse/core';
import { computed, useTemplateRef } from 'vue';
import { clock } from '../format';
import { CHART_SMALL, anyRan, bandsOf, chartModel, endAxis, figures, isoDay, size } from '../memory';
import { useMemoryDay } from '../memoryDay';
import { store, usedAccounts } from '../store';
import MemoryPlot from './MemoryPlot.vue';

const today = computed(() => isoDay(new Date(store.now)));
const { data, error } = useMemoryDay(today);

const all = computed(() =>
  store.state ? [...(store.state.main ? [store.state.main] : []), ...store.state.accounts] : [],
);
const bands = computed(() => (data.value ? bandsOf(data.value, usedAccounts(store.state), all.value) : []));
/** Every band: the card leaves none out. */
const every = computed(() => new Set(bands.value.map((b) => b.key)));
const figs = computed(() => (data.value ? figures(data.value, bands.value, every.value) : null));
/** Some AI session ran today: the figures and the chart show, else the line saying none is running. */
const ran = computed(() => !!figs.value && anyRan(figs.value.rows));
const model = computed(() => (data.value ? chartModel(data.value, bands.value, every.value, 2, 1) : null));

const box = useTemplateRef<HTMLElement>('box');
const { width: boxWidth } = useElementSize(box);
const width = computed(() => Math.max(0, Math.floor(boxWidth.value) - CHART_SMALL.gutter));
const axis = computed(() => (model.value ? endAxis(model.value, width.value) : null));

const now = computed(() => size(figs.value?.now ?? 0));
const peak = computed(() =>
  figs.value?.peak && figs.value.peakAt !== null ? { size: size(figs.value.peak), at: clock(figs.value.peakAt) } : null,
);
/** The chart link's name: what it shows. */
const label = computed(
  () =>
    `AI sessions' memory today: now ${now.value}` + (peak.value ? `, peak ${peak.value.size} at ${peak.value.at}` : ''),
);
</script>

<template>
  <section aria-labelledby="memory-heading" class="flex flex-col gap-3" data-memory-card>
    <div class="flex flex-wrap items-baseline gap-x-4 gap-y-1">
      <h2 id="memory-heading" class="text-16 font-semibold">
        <a
          href="#/memory"
          class="group inline-flex items-center gap-0.5 rounded-xs underline-offset-2 hov:underline press:text-secondary focus-visible:outline-2 focus-visible:outline-solid focus-visible:outline-offset-2 focus-visible:outline-indicator"
          >Memory<ChevronRight
            aria-hidden="true"
            class="size-4 text-icon group-hov:text-icon-hover group-press:text-text"
        /></a>
      </h2>
      <div
        v-if="ran && figs"
        aria-label="Memory today"
        class="ml-auto flex flex-wrap items-center gap-x-3.5 gap-y-1 text-12 text-secondary"
        data-memory-card-figures
      >
        <span class="inline-flex items-center gap-1.5" data-card-figure="now">
          <CpuAi aria-hidden="true" class="size-3.5 text-icon" />
          <span
            ><strong class="font-semibold text-text tabular-nums">{{ now }}</strong> now<span
              v-if="figs.nowShare"
              class="tabular-nums"
            >
              ({{ figs.nowShare }})</span
            ></span
          >
        </span>
        <span v-if="peak" class="inline-flex items-center gap-1.5" data-card-figure="peak">
          <Mountain aria-hidden="true" class="size-3.5 text-icon" />
          <span
            ><strong class="font-semibold text-text tabular-nums">{{ peak.size }}</strong> peak at
            <span class="tabular-nums">{{ peak.at }}</span></span
          >
        </span>
      </div>
    </div>

    <p v-if="error && !data" class="text-13 text-tertiary">
      {{ store.connected ? error : "Warden's core is not running." }}
    </p>
    <p v-else-if="!data" class="text-13 text-tertiary">Reading the day…</p>
    <p v-else-if="!ran" class="text-13 text-tertiary" data-card-none>No AI sessions are running.</p>
    <!-- The whole chart is one link to the Memory page, lit as a row that acts. -->
    <a
      v-else
      ref="box"
      href="#/memory"
      class="-mx-2 block rounded-lg p-2 hov:bg-row-hover press:bg-row-press focus-visible:outline-2 focus-visible:outline-solid focus-visible:outline-indicator"
      data-memory-card-chart
    >
      <!-- Its name, what it shows, in words; the drawing itself is hidden from screen readers. -->
      <span class="sr-only" data-chart-name>{{ label }}</span>
      <MemoryPlot
        v-if="model && axis"
        :model="model"
        :width="width"
        :chart="CHART_SMALL"
        :axis="axis"
        :hovered="null"
        :guide-x="null"
      />
    </a>
  </section>
</template>
