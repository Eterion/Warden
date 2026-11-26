<script setup lang="ts">
// Only a clipped label becomes a tooltip trigger. It uses the row's hover and pressed fills.
import WTooltip from '../components/WTooltip.vue';

defineOptions({ inheritAttrs: false });
defineProps<{ text: string; clipped: boolean }>();
</script>

<template>
  <WTooltip v-if="clipped" :text="text" as-child>
    <button
      v-bind="$attrs"
      type="button"
      class="block w-fit min-w-0 max-w-full cursor-pointer truncate rounded-sm text-left focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-indicator"
    >
      {{ text }}
    </button>
  </WTooltip>
  <span v-else v-bind="$attrs" class="block w-fit min-w-0 max-w-full truncate">{{ text }}</span>
</template>
