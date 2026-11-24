<script setup lang="ts">
// A table (ARIA roles on a grid, so columns line up without <table>): one bordered box, a header row, then the rows
// (WTableRow) with faint dividers. `cols` is the grid template and gap, shared by the header and every row through
// provide. With `bare` a widget supplies the frame. On the panel the box scrolls sideways and the caller's classes
// (a minimum width) go on the table inside.
// In a modal the header sits on the softer panel colour. Name the table with aria-label or aria-labelledby.
import { provide } from 'vue';
import { TABLE_COLS } from './table';

defineOptions({ inheritAttrs: false });
const props = withDefaults(defineProps<{ cols: string; on?: 'modal' | 'panel'; bare?: boolean }>(), { on: 'modal' });
provide(TABLE_COLS, () => props.cols);

const HEAD = 'grid items-center border-b border-divider-strong px-4 py-2 text-12 font-medium text-tertiary';
const FRAME = 'rounded-lg border border-divider-strong';
</script>

<template>
  <div v-if="props.on === 'panel'" class="overflow-x-auto" :class="!props.bare && FRAME">
    <div role="table" v-bind="$attrs">
      <div role="rowgroup">
        <div role="row" :class="[HEAD, props.cols, 'bg-head-bg']"><slot name="head" /></div>
      </div>
      <div role="rowgroup"><slot /></div>
    </div>
  </div>
  <div v-else role="table" v-bind="$attrs" :class="!props.bare && FRAME">
    <div role="rowgroup">
      <div role="row" :class="[HEAD, props.cols, 'bg-panel-soft', !props.bare && 'rounded-t-lg']">
        <slot name="head" />
      </div>
    </div>
    <div role="rowgroup"><slot /></div>
  </div>
</template>
