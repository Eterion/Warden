<script setup lang="ts">
// A session state's icon: a slow grey spinner while working, the amber hand while it waits for you, the coffee cup
// when idle, a check when done, and a dashed circle when its state is not known (seen only in a CLI's list).
import { CircleCheck, CircleDashed, Coffee, Hand, LoaderCircle } from 'lucide-vue-next';
import type { Shown } from '../sessions';
import { vSyncSpin } from './spin';

const props = withDefaults(defineProps<{ state: Shown; size?: 'sm' | 'md' }>(), { size: 'md' });
</script>

<template>
  <LoaderCircle
    v-if="props.state === 'working'"
    v-sync-spin
    :class="['shrink-0 animate-spin-slow text-icon', props.size === 'sm' ? 'size-3.25' : 'size-4']"
    :stroke-width="2.6"
    aria-hidden="true"
  />
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
  <CircleCheck
    v-else-if="props.state === 'done'"
    :class="['shrink-0 text-icon', props.size === 'sm' ? 'size-3.25' : 'size-4']"
    aria-hidden="true"
  />
  <CircleDashed
    v-else
    :class="['shrink-0 text-icon', props.size === 'sm' ? 'size-3.25' : 'size-4']"
    aria-hidden="true"
  />
</template>
