<script setup lang="ts">
// A provider's plain mark: Claude's orange spark, Codex's mark in the text colour (black in light, white in dark).
import claudeSpark from '../assets/icons/claude-spark-tight.png';
import type { Provider } from '../api';

const props = withDefaults(defineProps<{ provider: Provider; size?: 'xs' | 'sm' | 'md' | 'lg'; label?: string }>(), {
  size: 'md',
  label: undefined,
});

// Codex's mark is drawn a little smaller than Claude's so both read the same size.
const sizes = {
  xs: { claude: 'size-3', codex: 'size-3' },
  sm: { claude: 'size-4', codex: 'size-3.75' },
  md: { claude: 'size-5.5', codex: 'size-5' },
  lg: { claude: 'size-6', codex: 'size-5.5' },
} as const;
</script>

<template>
  <img
    v-if="props.provider === 'claude'"
    :src="claudeSpark"
    :alt="props.label ?? ''"
    :class="['block shrink-0', sizes[props.size].claude]"
  />
  <span
    v-else
    :role="props.label ? 'img' : undefined"
    :aria-label="props.label"
    :aria-hidden="props.label ? undefined : 'true'"
    :class="['codex-mark block shrink-0', sizes[props.size].codex]"
  ></span>
</template>
