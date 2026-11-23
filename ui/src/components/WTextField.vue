<script setup lang="ts">
// A one-line text field. Text is a name in medium weight; path is a folder or program path in mono; search adds the
// magnifier and a hidden label, and takes the caller's classes on that label. The caller sets the width.
import { Search } from '@lucide/vue';
import { vLabelState } from './labels';

defineOptions({ inheritAttrs: false });
const model = defineModel<string>({ required: true });
const props = withDefaults(defineProps<{ kind?: 'text' | 'path' | 'search'; label?: string }>(), {
  kind: 'text',
  label: undefined,
});

const BASE =
  'h-9 rounded-lg border border-control bg-modal text-text hov:border-secondary press:border-secondary focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-indicator disabled:opacity-55';
const KIND = {
  text: 'px-2.5 text-14 font-medium',
  path: 'px-2.5 font-mono text-12 placeholder:font-sans placeholder:text-tertiary',
  search: 'w-full pr-2.5 pl-8 text-14 placeholder:text-tertiary',
};
</script>

<template>
  <label v-if="props.kind === 'search'" :class="['relative block', $attrs.class]">
    <span class="sr-only">{{ props.label }}</span>
    <Search
      class="pointer-events-none absolute top-1/2 left-2.5 size-3.75 -translate-y-1/2 text-icon"
      aria-hidden="true"
    />
    <input
      v-model="model"
      v-label-state
      v-bind="{ ...$attrs, class: undefined }"
      type="search"
      :class="[BASE, KIND.search]"
    />
  </label>
  <input
    v-else
    v-model="model"
    v-label-state
    type="text"
    :spellcheck="props.kind === 'path' ? false : undefined"
    v-bind="$attrs"
    :class="[BASE, KIND[props.kind]]"
  />
</template>
