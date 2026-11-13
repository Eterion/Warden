<script setup lang="ts">
// A row of a `WList`. Plain, it holds whatever the caller lays out (wrapping when narrow); with `action` the whole row
// is a button that emits `select`, a little taller, lit on hover.
const props = defineProps<{ action?: boolean }>();
const emit = defineEmits<{ select: [] }>();
</script>

<template>
  <li class="border-b border-divider last:border-b-0">
    <button
      v-if="props.action"
      type="button"
      class="flex w-full cursor-pointer items-center gap-3 px-4 py-3.5 text-left hov:bg-row-hover press:bg-row-press focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-indicator"
      @click="emit('select')"
    >
      <slot />
    </button>
    <div v-else class="flex flex-wrap items-center gap-3 px-4 py-3">
      <slot />
    </div>
  </li>
</template>
