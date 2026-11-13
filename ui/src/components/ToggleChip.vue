<script setup lang="ts">
// A chip that is on or off (aria-pressed). Filled shows a check when on, for a set of options that each count;
// outline is a quieter pick from a list, its text cut short to fit.
import { Check } from 'lucide-vue-next';

const props = withDefaults(defineProps<{ pressed: boolean; look?: 'filled' | 'outline' }>(), { look: 'filled' });
</script>

<template>
  <button
    v-if="props.look === 'filled'"
    type="button"
    :aria-pressed="props.pressed"
    class="group inline-flex h-8 cursor-pointer items-center gap-1.5 rounded-lg bg-chip-off pr-3 pl-2.25 text-14 font-medium text-chip-off-text hov:bg-press press:bg-press-strong focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-indicator aria-pressed:bg-chip-on aria-pressed:text-chip-on-text aria-pressed:hov:bg-primary-hover aria-pressed:press:bg-primary-press"
  >
    <Check class="size-3.5 opacity-0 group-aria-pressed:opacity-100" :stroke-width="2.6" aria-hidden="true" />
    <slot />
  </button>
  <button
    v-else
    type="button"
    :aria-pressed="props.pressed"
    class="inline-flex h-7 max-w-full cursor-pointer items-center rounded-md px-2.5 text-12 text-chip-off-text inset-ring-1 inset-ring-ring-strong hov:bg-chip-off press:bg-press focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-indicator aria-pressed:bg-chip-on aria-pressed:text-chip-on-text aria-pressed:inset-ring-0 aria-pressed:hov:bg-primary-hover aria-pressed:press:bg-primary-press"
  >
    <span class="truncate"><slot /></span>
  </button>
</template>
