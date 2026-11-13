<script setup lang="ts">
// A usage bar: green below 50%, orange from 50%, red from 80%, and grey while a Warden account's login is idle
// (`idle`). The value is in the text beside it. An optional `mark` (a percent) draws a thin tick across the bar, taller
// than it, where main hands work off.
import type { Level } from '../format';

const props = withDefaults(
  defineProps<{ percent: number; level: Level | 'idle'; track?: 'panel' | 'win'; mark?: number }>(),
  { track: 'panel', mark: undefined },
);
const fill = { low: 'bg-bar-low', mid: 'bg-bar-mid', high: 'bg-bar-high', idle: 'bg-bar-idle' } as const;
</script>

<template>
  <span
    aria-hidden="true"
    :class="['relative flex shrink-0 items-center', props.track === 'win' ? 'w-sidebar-bar' : 'w-full']"
  >
    <span
      :class="['block h-bar w-full overflow-hidden rounded-full', props.track === 'win' ? 'bg-track-win' : 'bg-track']"
    >
      <span :class="['block h-full rounded-full', fill[props.level]]" :style="{ width: `${props.percent}%` }"></span>
    </span>
    <span
      v-if="props.mark !== undefined"
      data-mark
      class="absolute h-3 w-0.5 -translate-x-1/2 rounded-full bg-bar-mark"
      :style="{ left: `${props.mark}%` }"
    ></span>
  </span>
</template>
