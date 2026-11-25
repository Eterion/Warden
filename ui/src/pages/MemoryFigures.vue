<script setup lang="ts">
import { CpuAi, Monitor, Mountain } from '@lucide/vue';
import { computed } from 'vue';
import OwlIcon from '../components/OwlIcon.vue';
import WTooltip from '../components/WTooltip.vue';
import WWidget from '../components/WWidget.vue';
import type { MemoryDay } from '../api';
import {
  bandAt,
  memoryTrend,
  meterParts,
  shareText,
  size,
  wardenMemory,
  type ChartModel,
  type Figures,
} from '../memory';
import { clock } from '../format';
import MemoryTrend from './MemoryTrend.vue';

const props = defineProps<{
  figures: Figures;
  today: boolean;
  total: number | null;
  model: ChartModel | null;
  day: MemoryDay;
}>();

const SHARE_NOTE =
  'Shares count private memory, part of which Windows can page out to disk, so a share can read a little high when memory is short.';

const meter = computed(() => (props.figures.computer ? meterParts(props.figures.computer) : null));
const trend = computed(() => {
  const model = props.model;
  return props.today && model
    ? memoryTrend(model.minutes, (m) => model.bands.reduce((sum, b) => sum + bandAt(b, m), 0))
    : null;
});
const wardenTrend = computed(() => {
  const valueAt = wardenMemory(props.day);
  return valueAt ? memoryTrend(props.day.minutes, valueAt) : null;
});
const ICON = 'size-4 shrink-0 text-icon';
const BODY = 'flex flex-1 flex-col gap-2';
const VALUE = 'flex flex-wrap items-baseline gap-x-2 tabular-nums';
</script>

<template>
  <div class="grid grid-cols-figures gap-4" data-figures>
    <WWidget v-if="props.today" group title="AI sessions now" data-figure="now">
      <template #icon><CpuAi :class="ICON" aria-hidden="true" /></template>
      <template #body>
        <div :class="BODY">
          <p :class="VALUE">
            <strong class="text-24 font-semibold">{{ size(props.figures.now ?? 0) }}</strong>
            <WTooltip
              v-if="props.figures.nowShare && props.total"
              :text="SHARE_NOTE"
              class="text-left text-13 text-secondary"
              data-share-note
            >
              {{ props.figures.nowShare }} of {{ size(props.total) }}
            </WTooltip>
          </p>
          <MemoryTrend v-if="trend" :paths="trend" class="mt-auto" />
        </div>
      </template>
    </WWidget>

    <WWidget group :title="props.today ? 'Peak today' : 'Peak'" data-figure="peak">
      <template #icon><Mountain :class="ICON" aria-hidden="true" /></template>
      <template #body>
        <p :class="VALUE">
          <strong class="text-24 font-semibold">{{ size(props.figures.peak ?? 0) }}</strong>
          <span v-if="props.figures.peak && props.figures.peakAt !== null" class="text-13 text-secondary">
            at {{ clock(props.figures.peakAt) }}
          </span>
        </p>
      </template>
      <template v-if="props.figures.peak !== null && props.total" #footer>
        <p class="text-12 text-tertiary">{{ shareText(props.figures.peak, props.total) }} of the computer's memory</p>
      </template>
    </WWidget>

    <WWidget v-if="props.figures.computer && meter" group title="Computer" data-figure="computer" data-computer>
      <template #icon><Monitor :class="ICON" aria-hidden="true" /></template>
      <template #body>
        <div :class="BODY">
          <p :class="VALUE">
            <strong :class="['text-24 font-semibold', props.figures.computer.high && 'text-attn']">
              {{ size(props.figures.computer.used) }}
            </strong>
            <span :class="['text-13', props.figures.computer.high ? 'font-medium text-attn' : 'text-secondary']">
              {{ shareText(props.figures.computer.used, props.figures.computer.total) }} of
              {{ size(props.figures.computer.total) }}{{ props.today ? '' : ` at ${clock(props.figures.computer.at)}` }}
            </span>
          </p>
          <p class="sr-only" data-meter-text>
            AI sessions {{ size(props.figures.computer.ai) }} of it ({{
              shareText(props.figures.computer.ai, props.figures.computer.total)
            }})
          </p>
          <div class="mt-auto flex flex-col gap-1" aria-hidden="true">
            <div
              class="flex h-1.5 w-full max-w-40 gap-px overflow-hidden rounded-full bg-track"
              data-meter
              :data-high="props.figures.computer.high || undefined"
            >
              <span v-if="meter.ai > 0" class="h-full bg-text" :style="{ width: `${meter.ai}%` }" data-meter-ai></span>
              <span
                v-if="meter.rest > 0"
                :class="['h-full', props.figures.computer.high ? 'bg-attn-icon' : 'bg-control']"
                :style="{ width: `${meter.rest}%` }"
                data-meter-used
              ></span>
            </div>
            <div class="flex flex-wrap gap-x-3 gap-y-1 text-12 text-tertiary" data-computer-legend>
              <span class="inline-flex items-center gap-1.5"
                ><span class="size-2.5 rounded-xs bg-text"></span>AI sessions</span
              >
              <span class="inline-flex items-center gap-1.5">
                <span
                  :class="['size-2.5 rounded-xs', props.figures.computer.high ? 'bg-attn-icon' : 'bg-control']"
                ></span>
                Everything else
              </span>
            </div>
          </div>
        </div>
      </template>
    </WWidget>

    <WWidget v-if="props.figures.warden !== null" group title="Warden itself" data-figure="warden">
      <template #icon><OwlIcon :class="ICON" :size="16" aria-hidden="true" /></template>
      <template #body>
        <div :class="BODY">
          <p :class="VALUE">
            <strong class="text-24 font-semibold">{{ size(props.figures.warden) }}</strong>
            <span v-if="props.total || !props.today" class="text-13 text-secondary">
              <template v-if="props.total"
                >{{ shareText(props.figures.warden, props.total) }} of {{ size(props.total) }}</template
              >
              <template v-if="!props.today">{{ props.total ? ' at its highest' : 'At its highest' }}</template>
            </span>
          </p>
          <p v-if="props.figures.wardenPeak" class="sr-only" data-warden-peak-text>
            Highest {{ size(props.figures.wardenPeak.used) }} at {{ clock(props.figures.wardenPeak.at) }}
          </p>
          <MemoryTrend v-if="wardenTrend" :paths="wardenTrend" class="mt-auto" />
        </div>
      </template>
    </WWidget>
  </div>
</template>
