<script setup lang="ts">
// The "now" chip beside an account's sparkline: its current sessions as state icon and count, amber when one waits
// for you. Nothing when there are no sessions.
import { computed } from 'vue';
import { nowLabel, type Tally } from '../sessions';
import StateIcon from './StateIcon.vue';

const props = defineProps<{ tally: Tally }>();
const label = computed(() => nowLabel(props.tally));
const items = computed(() =>
  (['waiting', 'working', 'idle'] as const)
    .filter((s) => props.tally[s] > 0)
    .map((s) => ({ state: s, n: props.tally[s] })),
);
</script>

<template>
  <span
    v-if="label"
    role="img"
    :aria-label="label"
    :title="label"
    :class="[
      'inline-flex h-6 items-center gap-2.5 rounded-full px-2.25 text-12 font-semibold whitespace-nowrap tabular-nums ring-1 ring-inset',
      props.tally.waiting ? 'bg-attn-bg text-attn ring-attn-ring' : 'bg-panel text-text ring-divider-strong',
    ]"
  >
    <span v-for="x in items" :key="x.state" class="inline-flex items-center gap-1">
      <StateIcon :state="x.state" size="sm" />{{ x.n }}
    </span>
  </span>
</template>
