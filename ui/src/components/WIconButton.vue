<script setup lang="ts">
// A button that is only an icon, named by its label: md beside a heading, sm (a step smaller) on a row. It also serves
// as a Reka trigger (as-child), which marks it open with data-state.
import type { Component } from 'vue';

const props = withDefaults(defineProps<{ label: string; icon: Component; size?: 'md' | 'sm' }>(), { size: 'md' });
</script>

<template>
  <button
    type="button"
    :aria-label="props.label"
    :class="[
      'inline-flex shrink-0 cursor-pointer items-center justify-center rounded-lg text-icon hov:bg-chip-off press:bg-press focus-visible:outline-2 focus-visible:outline-indicator data-[state=open]:bg-chip-off data-[state=open]:press:bg-press',
      props.size === 'md' ? 'size-9' : 'size-8',
    ]"
  >
    <component :is="props.icon" class="size-4.5" aria-hidden="true" />
  </button>
</template>
