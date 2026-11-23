<script setup lang="ts">
// A session state's icon: a slow grey spinner while working (a still, thin ring with a darker arc turning on it), the
// amber hand while it waits for you, the coffee cup when idle, a muted dash when done, and a muted ghost when its state
// is not known (seen only in a CLI's list).
import { Circle, Coffee, Ghost, Hand, LoaderCircle, Minus } from '@lucide/vue';
import type { Shown } from '../sessions';
import { vSyncSpin } from './spin';

const props = withDefaults(defineProps<{ state: Shown; size?: 'sm' | 'md' }>(), { size: 'md' });
</script>

<template>
  <!-- Lucide draws Circle at r 10 and LoaderCircle's arc at r 9: the track is scaled to 90% so both share one circle,
       and its line is set so it stays thinner than the arc after the scale. -->
  <span
    v-if="props.state === 'working'"
    :class="['relative inline-flex shrink-0', props.size === 'sm' ? 'size-3.25' : 'size-4']"
    aria-hidden="true"
  >
    <Circle data-ring-track class="absolute inset-0 size-full scale-90 text-track-win" :stroke-width="1.6" />
    <LoaderCircle v-sync-spin class="relative size-full animate-spin-slow text-icon" :stroke-width="2.6" />
  </span>
  <Hand
    v-else-if="props.state === 'waiting'"
    :class="['shrink-0 text-attn-icon', props.size === 'sm' ? 'size-3.25' : 'size-4']"
    aria-hidden="true"
  />
  <Coffee
    v-else-if="props.state === 'idle'"
    :class="['shrink-0 text-act-idle', props.size === 'sm' ? 'size-3.25' : 'size-4']"
    aria-hidden="true"
  />
  <Minus
    v-else-if="props.state === 'done'"
    :class="['shrink-0 text-icon-faded', props.size === 'sm' ? 'size-3.25' : 'size-4']"
    aria-hidden="true"
  />
  <Ghost
    v-else
    :class="['shrink-0 text-icon-faded', props.size === 'sm' ? 'size-3.25' : 'size-4']"
    aria-hidden="true"
  />
</template>
