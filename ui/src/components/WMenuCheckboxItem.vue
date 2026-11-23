<script setup lang="ts">
// A checkbox in a dropdown menu: a check box (filled with a check when on, an outline when off, as the Overview's
// account checkboxes), an optional mark after it (`mark` slot, an account's provider mark), its label, which wraps
// within the menu when it is long, and an optional count at the end (`end` slot, small, secondary). Choosing it keeps the menu open, so several can be ticked in a row. Keyboard focus
// shows the focus outline inside the row.
import { Check } from '@lucide/vue';
import { DropdownMenuCheckboxItem } from 'reka-ui';

const checked = defineModel<boolean>({ required: true });
</script>

<template>
  <DropdownMenuCheckboxItem
    v-model="checked"
    class="group flex min-h-8 cursor-pointer items-center gap-2.5 rounded-md px-2.5 py-1.5 select-none focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-indicator data-[highlighted]:bg-chip-off data-[highlighted]:press:bg-press"
    @select="$event.preventDefault()"
  >
    <span
      :class="[
        'inline-flex size-4 shrink-0 items-center justify-center rounded-sm',
        checked
          ? 'bg-primary text-on-primary group-hov:bg-primary-hover group-press:bg-primary-press'
          : 'border-2 border-control',
      ]"
      aria-hidden="true"
    >
      <Check v-if="checked" class="size-3" :stroke-width="3" />
    </span>
    <span v-if="$slots.mark" class="inline-flex shrink-0"><slot name="mark" /></span>
    <span class="min-w-0 wrap-anywhere"><slot /></span>
    <span v-if="$slots.end" class="ml-auto shrink-0 pl-4 text-12 text-secondary tabular-nums"><slot name="end" /></span>
  </DropdownMenuCheckboxItem>
</template>
