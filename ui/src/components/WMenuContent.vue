<script setup lang="ts">
// A dropdown menu's panel: as wide as its longest item, within the window (taller than the room left, it scrolls),
// opening and closing with a short soft scale (a quick fade with reduced motion). Put WMenuItem or WMenuCheckboxItem
// items inside, within a Reka DropdownMenuRoot.
import { DropdownMenuContent } from 'reka-ui';

const props = withDefaults(defineProps<{ align?: 'start' | 'end' }>(), { align: 'end' });
</script>

<template>
  <DropdownMenuContent
    :align="props.align"
    :side-offset="4"
    :collision-padding="8"
    class="z-50 max-h-menu max-w-menu origin-menu overflow-y-auto rounded-lg bg-modal p-1 text-14 text-text shadow-modal data-[state=closed]:animate-menu-out data-[state=open]:animate-menu-in"
  >
    <slot />
  </DropdownMenuContent>
</template>
