<script setup lang="ts" generic="T extends string">
// A segmented control: one choice from a few, as a radio group.
import { RadioGroupItem, RadioGroupRoot } from 'reka-ui';
import type { Component } from 'vue';

const model = defineModel<T>({ required: true });
const props = defineProps<{
  label: string;
  options: { value: T; label: string; icon?: Component }[];
  full?: boolean;
}>();
</script>

<template>
  <RadioGroupRoot
    v-model="model"
    :aria-label="props.label"
    orientation="horizontal"
    :class="['gap-0.5 rounded-lg bg-seg-track p-0.5', props.full ? 'grid grid-cols-2' : 'inline-flex']"
  >
    <RadioGroupItem
      v-for="o in props.options"
      :key="o.value"
      :value="o.value"
      class="inline-flex h-7.5 cursor-pointer items-center justify-center gap-1.5 rounded-md px-3 text-13 font-medium whitespace-nowrap text-seg-text hov:bg-press press:bg-press-strong press:text-seg-sel-text focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-indicator data-[state=checked]:bg-seg-sel data-[state=checked]:text-seg-sel-text data-[state=checked]:shadow-seg data-[state=checked]:hov:bg-seg-sel-hover data-[state=checked]:press:bg-press"
    >
      <component :is="o.icon" v-if="o.icon" class="size-3.5" aria-hidden="true" />
      {{ o.label }}
    </RadioGroupItem>
  </RadioGroupRoot>
</template>
