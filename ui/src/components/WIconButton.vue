<script setup lang="ts">
// A button that is only an icon, named by its label: md beside a heading, sm (a step smaller) on a row, xs a bare
// small icon beside a small heading (the sidebar's Accounts), lit by its colour alone, with no background. It also
// serves as a Reka trigger (as-child), which marks it open with data-state. One that cannot be used now says why
// (`disabledReason`): it stays focusable with `aria-disabled`, shows the reason in the shared tooltip on hover and
// focus, does nothing when activated, and has no hover or pressed state; only its icon fades.
import { computed, useAttrs, type Component } from 'vue';
import WTooltip from './WTooltip.vue';

defineOptions({ inheritAttrs: false });
const props = withDefaults(
  defineProps<{ label: string; icon: Component; size?: 'md' | 'sm' | 'xs'; disabledReason?: string }>(),
  { size: 'md', disabledReason: undefined },
);

/** What a disabled one passes on: everything but its click handler, so activating it does nothing. */
const attrs = useAttrs();
const inert = computed(() => Object.fromEntries(Object.entries(attrs).filter(([k]) => k !== 'onClick')));

const BASE =
  'inline-flex shrink-0 cursor-pointer items-center justify-center text-icon focus-visible:outline-2 focus-visible:outline-indicator';
const SHAPE = { md: 'size-9 rounded-lg', sm: 'size-8 rounded-lg', xs: 'size-6 rounded-lg' };
const LOOK = {
  md: 'hov:bg-chip-off press:bg-press data-[state=open]:bg-chip-off data-[state=open]:press:bg-press',
  sm: 'hov:bg-chip-off press:bg-press data-[state=open]:bg-chip-off data-[state=open]:press:bg-press',
  xs: 'hov:text-icon-hover press:text-text data-[state=open]:text-text',
};
</script>

<template>
  <WTooltip v-if="props.disabledReason" :text="props.disabledReason" as-child>
    <button
      v-bind="inert"
      type="button"
      :aria-label="props.label"
      aria-disabled="true"
      :class="[BASE, SHAPE[props.size], 'cursor-not-allowed']"
    >
      <component
        :is="props.icon"
        :class="[props.size === 'xs' ? 'size-3.5' : 'size-4.5', 'opacity-40']"
        aria-hidden="true"
      />
    </button>
  </WTooltip>
  <button
    v-else
    v-bind="$attrs"
    type="button"
    :aria-label="props.label"
    :class="[BASE, SHAPE[props.size], LOOK[props.size]]"
  >
    <component :is="props.icon" :class="props.size === 'xs' ? 'size-3.5' : 'size-4.5'" aria-hidden="true" />
  </button>
</template>
