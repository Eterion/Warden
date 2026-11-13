<script setup lang="ts">
// An item in a dropdown menu: a Lucide icon and a label on one line. `danger` marks a destructive action (Remove) in
// red, which keeps its contrast on hover and when pressed.
import { DropdownMenuItem } from 'reka-ui';
import type { Component } from 'vue';

const props = defineProps<{ icon: Component; danger?: boolean; disabled?: boolean }>();
const emit = defineEmits<{ select: [event: Event] }>();
</script>

<template>
  <DropdownMenuItem
    :disabled="props.disabled"
    :class="[
      'flex h-8 cursor-pointer items-center gap-2.5 rounded-md px-2.5 whitespace-nowrap outline-none select-none data-[disabled]:cursor-default data-[disabled]:text-tertiary data-[highlighted]:bg-chip-off data-[highlighted]:press:bg-press',
      props.danger && 'text-danger',
    ]"
    @select="emit('select', $event)"
  >
    <component
      :is="props.icon"
      :class="['size-4 shrink-0', props.danger ? 'text-danger' : 'text-icon']"
      aria-hidden="true"
    />
    <slot />
  </DropdownMenuItem>
</template>
