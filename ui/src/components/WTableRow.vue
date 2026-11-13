<script setup lang="ts">
// A row of a WTable, on the table's grid, with a faint divider above all but the first. The caller sets the row's
// height (its vertical padding) and any state (selected, dragged) through its classes. `off` marks a switched-off
// account's row with a subtle grey background.
import { inject } from 'vue';
import { TABLE_COLS } from './table';

const props = defineProps<{ off?: boolean }>();
const cols = inject(TABLE_COLS, () => '');
</script>

<template>
  <div
    role="row"
    :class="[
      'grid items-center border-t border-divider px-4 first:border-t-0 last:rounded-b-lg',
      props.off && 'bg-row-off',
      cols(),
    ]"
  >
    <slot />
  </div>
</template>
