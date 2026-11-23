<script setup lang="ts">
// The Memory page's figures, stretched across the width in equal columns (fewer on a narrow page), in one rounded
// bordered box, as the tables, with thin dividers between. Each has its icon (32 px, a 1.5 stroke, in the secondary text colour), then a
// small label, the number and a muted detail: the AI sessions now (today) and their share of the computer, the peak
// and its time, the computer's memory in use of its total with a slim meter (the AI sessions' part of it marked in the
// text colour; the bar, the number and the share amber above 90% in use), and Warden itself.
import { CpuAi, Monitor, Mountain } from '@lucide/vue';
import { computed } from 'vue';
import OwlIcon from '../components/OwlIcon.vue';
import WTooltip from '../components/WTooltip.vue';
import { meterParts, shareText, size, type Figures } from '../memory';
import { clock } from '../format';

const props = defineProps<{ figures: Figures; today: boolean; total: number | null }>();

/** Why a share can read a little high. */
const SHARE_NOTE =
  'Shares count private memory, part of which Windows can page out to disk, so a share can read a little high when memory is short.';

const meter = computed(() => (props.figures.computer ? meterParts(props.figures.computer) : null));
const ICON = 'absolute top-1/2 left-4 size-8 -translate-y-1/2 text-secondary';
</script>

<template>
  <div class="overflow-hidden rounded-lg border border-divider-strong">
    <dl class="-mt-px -ml-px grid grid-cols-figures" data-figures>
      <div v-if="props.today" class="relative border-t border-l border-divider py-3 pr-4 pl-16" data-figure="now">
        <dt class="text-12 text-secondary">
          <CpuAi :class="ICON" :stroke-width="1.5" aria-hidden="true" />AI sessions now
        </dt>
        <dd class="text-16 font-semibold tabular-nums">{{ size(props.figures.now ?? 0) }}</dd>
        <dd v-if="props.total" class="text-12 text-tertiary tabular-nums">
          <!-- Said once, where the share first shows. -->
          <WTooltip :text="SHARE_NOTE" class="text-left" data-share-note
            >{{ props.figures.nowShare }} of {{ size(props.total) }}</WTooltip
          >
        </dd>
      </div>
      <div class="relative border-t border-l border-divider py-3 pr-4 pl-16" data-figure="peak">
        <dt class="text-12 text-secondary">
          <Mountain :class="ICON" :stroke-width="1.5" aria-hidden="true" />{{ props.today ? 'Peak today' : 'Peak' }}
        </dt>
        <dd class="text-16 font-semibold tabular-nums">{{ size(props.figures.peak ?? 0) }}</dd>
        <dd v-if="props.figures.peakAt !== null && props.figures.peak" class="text-12 text-tertiary tabular-nums">
          at {{ clock(props.figures.peakAt) }}
        </dd>
      </div>
      <div
        v-if="props.figures.computer && meter"
        class="relative border-t border-l border-divider py-3 pr-4 pl-16"
        data-figure="computer"
        data-computer
      >
        <dt class="text-12 text-secondary"><Monitor :class="ICON" :stroke-width="1.5" aria-hidden="true" />Computer</dt>
        <dd :class="['text-16 font-semibold tabular-nums', props.figures.computer.high && 'text-attn']">
          {{ size(props.figures.computer.used) }}
        </dd>
        <!-- The meter: the memory in use, the AI sessions' part of it first, in the text colour; said in words for
             screen readers, in the same minute as the figure. -->
        <dd class="sr-only" data-meter-text>
          AI sessions {{ size(props.figures.computer.ai) }} of it ({{
            shareText(props.figures.computer.ai, props.figures.computer.total)
          }})
        </dd>
        <dd
          aria-hidden="true"
          class="my-1 flex h-1.5 w-full max-w-40 gap-px overflow-hidden rounded-full bg-track"
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
        </dd>
        <dd :class="['text-12 tabular-nums', props.figures.computer.high ? 'font-medium text-attn' : 'text-tertiary']">
          {{ shareText(props.figures.computer.used, props.figures.computer.total) }} of
          {{ size(props.figures.computer.total) }}{{ props.today ? '' : ` at ${clock(props.figures.computer.at)}` }}
        </dd>
      </div>
      <div
        v-if="props.figures.warden !== null"
        class="relative border-t border-l border-divider py-3 pr-4 pl-16"
        data-figure="warden"
      >
        <dt class="text-12 text-secondary">
          <OwlIcon :class="ICON" :size="32" :stroke-width="1.5" aria-hidden="true" />Warden itself
        </dt>
        <dd class="text-16 font-semibold tabular-nums">{{ size(props.figures.warden) }}</dd>
        <dd v-if="!props.today" class="text-12 text-tertiary">at its highest</dd>
        <dd v-else-if="props.total" class="text-12 text-tertiary tabular-nums">
          {{ shareText(props.figures.warden, props.total) }} of {{ size(props.total) }}
        </dd>
      </div>
    </dl>
  </div>
</template>
