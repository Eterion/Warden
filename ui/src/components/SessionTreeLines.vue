<script setup lang="ts">
// Decorative lines for nested session rows. Memory's first rail meets its disclosure button above.
defineProps<{ depth: number; rails: boolean[]; last: boolean; disclosure?: boolean }>();
</script>

<template>
  <span
    v-for="level in depth"
    :key="level"
    aria-hidden="true"
    class="relative shrink-0 self-stretch"
    :class="disclosure ? 'w-12' : 'w-6'"
  >
    <span
      v-if="level < depth ? rails[level - 1] : !last"
      data-line="rail"
      class="absolute -top-2.5 -bottom-2.25 border-l border-divider-strong"
      :class="disclosure ? 'left-3' : 'left-2'"
    ></span>
    <span
      v-if="level === depth && !last"
      data-line="stub"
      class="absolute bottom-1/2 border-b border-divider-strong"
      :class="disclosure ? 'left-3.25 w-7.75' : 'left-2.25 w-2.75'"
    ></span>
    <span
      v-else-if="level === depth"
      data-line="elbow"
      class="absolute -top-2.5 bottom-1/2 rounded-bl-sm border-b border-l border-divider-strong"
      :class="disclosure ? 'left-3 w-8' : 'left-2 w-3'"
    ></span>
  </span>
</template>
