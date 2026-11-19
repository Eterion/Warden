<script setup lang="ts">
// A button that is only an icon, named by its label: md beside a heading, sm (a step smaller) on a row, xs a bare
// small icon beside a small heading (the sidebar's Accounts), lit by its colour alone, with no background. It also
// serves as a Reka trigger (as-child), which marks it open with data-state.
import type { Component } from 'vue';

const props = withDefaults(defineProps<{ label: string; icon: Component; size?: 'md' | 'sm' | 'xs' }>(), {
  size: 'md',
});

const LOOK = {
  md: 'size-9 rounded-lg hov:bg-chip-off press:bg-press data-[state=open]:bg-chip-off data-[state=open]:press:bg-press',
  sm: 'size-8 rounded-lg hov:bg-chip-off press:bg-press data-[state=open]:bg-chip-off data-[state=open]:press:bg-press',
  xs: 'size-6 rounded-lg hov:text-icon-hover press:text-text data-[state=open]:text-text',
};
</script>

<template>
  <button
    type="button"
    :aria-label="props.label"
    :class="[
      'inline-flex shrink-0 cursor-pointer items-center justify-center text-icon focus-visible:outline-2 focus-visible:outline-indicator',
      LOOK[props.size],
    ]"
  >
    <component :is="props.icon" :class="props.size === 'xs' ? 'size-3.5' : 'size-4.5'" aria-hidden="true" />
  </button>
</template>
