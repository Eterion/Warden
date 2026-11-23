<script setup lang="ts">
// A chip that a click removes; the label names that action. md is an item the user added, shown on like a pressed
// toggle chip; sm is a filter in force, beside a heading.
import { Check, X } from '@lucide/vue';

const props = withDefaults(defineProps<{ label: string; size?: 'md' | 'sm' }>(), { size: 'md' });
</script>

<template>
  <button
    v-if="props.size === 'md'"
    type="button"
    :aria-label="props.label"
    class="inline-flex h-8 max-w-full cursor-pointer items-center gap-1.5 rounded-lg bg-chip-on pr-2.5 pl-2.25 text-14 font-medium text-chip-on-text hov:bg-primary-hover press:bg-primary-press focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-indicator"
  >
    <Check class="size-3.5 shrink-0" :stroke-width="2.6" aria-hidden="true" />
    <span class="truncate"><slot /></span>
    <X class="size-3.5 shrink-0" aria-hidden="true" />
  </button>
  <button
    v-else
    type="button"
    :aria-label="props.label"
    class="inline-flex h-6 cursor-pointer items-center gap-1.5 rounded-full bg-tag-bg pr-2 pl-2.5 text-12 font-medium text-text hov:bg-press press:bg-press-strong focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-indicator"
  >
    <slot />
    <X class="size-3 text-icon" :stroke-width="2.4" aria-hidden="true" />
  </button>
</template>
