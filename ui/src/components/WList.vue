<script setup lang="ts">
// A plain list with faint dividers between rows. A widget supplies the frame for a bare list.
const props = defineProps<{ bare?: boolean }>();
</script>

<template>
  <ul class="overflow-hidden" :class="!props.bare && 'rounded-lg border border-divider-strong'">
    <slot />
  </ul>
</template>
