<script setup lang="ts">
// "Last 2 hours, then now": one solid step line of how many sessions were working every 5 minutes, with a light
// wash under it and amber dots where a session started waiting for you. No chart library.
import { computed } from 'vue';
import { SPARK, sparkPaths, type Point } from '../sessions';

const props = defineProps<{ points: Point[]; most: number; name: string }>();
const paths = computed(() => sparkPaths(props.points, props.most, props.name));
</script>

<template>
  <svg
    :viewBox="`0 0 ${SPARK.width} ${SPARK.height}`"
    role="img"
    :aria-label="paths.label"
    class="block h-7 w-sparkline shrink-0 overflow-visible"
  >
    <title>{{ paths.label }}</title>
    <path :d="paths.area" class="fill-act-working stroke-none" fill-opacity="0.1" />
    <path
      :d="paths.line"
      class="fill-none stroke-act-working"
      stroke-width="1.5"
      stroke-linejoin="round"
      stroke-linecap="round"
    />
    <circle
      v-for="(d, i) in paths.dots"
      :key="i"
      :cx="d.x"
      :cy="d.y"
      r="2.5"
      class="fill-attn-icon stroke-panel"
      stroke-width="1.5"
    />
  </svg>
</template>
