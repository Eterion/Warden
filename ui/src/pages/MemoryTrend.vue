<script setup lang="ts">
defineProps<{ paths: { line: string; area: string } }>();
</script>

<template>
  <svg aria-hidden="true" class="h-6 w-full" viewBox="0 0 100 24" preserveAspectRatio="none" data-memory-trend>
    <path :d="paths.area" class="fill-secondary opacity-10" />
    <path :d="paths.line" class="fill-none stroke-secondary" stroke-width="1.5" vector-effect="non-scaling-stroke" />
  </svg>
</template>
