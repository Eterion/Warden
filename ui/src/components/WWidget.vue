<script setup lang="ts">
// A bordered widget with its title and controls above its content. A section gets a heading, and a group gets a
// plain label. The body slot adds padding for figures and charts, and the default slot fits a table or list.
// The footer slot adds an edge-to-edge divider and equal vertical padding.
// A linked widget has one link over the frame, with the content free of controls.
import { useId, useTemplateRef } from 'vue';

const props = defineProps<{
  title: string;
  headingId?: string;
  focusableHeading?: boolean;
  centerHeader?: boolean;
  group?: boolean;
  href?: string;
  label?: string;
}>();
const id = useId();
const heading = useTemplateRef<HTMLElement>('heading');
defineExpose({ heading });
</script>

<template>
  <component
    :is="props.group ? 'div' : 'section'"
    :role="props.group ? 'group' : undefined"
    :aria-labelledby="props.headingId ?? id"
    class="flex min-w-0 flex-col"
  >
    <div
      :class="[
        'flex flex-1 flex-col rounded-lg border border-divider-strong',
        props.href && 'relative hov:bg-row-hover press:bg-row-press',
      ]"
    >
      <header
        class="flex flex-wrap gap-x-4 gap-y-2 px-4 pt-3"
        :class="[props.centerHeader ? 'items-center' : 'items-baseline', $slots.icon ? 'pb-1' : 'pb-3']"
      >
        <component
          :is="props.group ? 'span' : 'h2'"
          :id="props.headingId ?? id"
          ref="heading"
          :tabindex="props.focusableHeading ? -1 : undefined"
          :class="[
            $slots.icon ? 'flex items-center gap-2 text-13 font-medium text-secondary' : 'text-16 font-semibold',
            props.focusableHeading &&
              'rounded-xs outline-none focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-indicator',
          ]"
        >
          <slot name="icon" />{{ props.title }}{{ $slots['title-after'] ? ' ' : '' }}<slot name="title-after" />
        </component>
        <slot name="header" />
      </header>
      <div v-if="$slots.body" class="flex flex-1 flex-col px-4" :class="$slots.footer ? 'pb-3' : 'pb-4'">
        <slot name="body" />
      </div>
      <slot v-else />
      <div v-if="$slots.footer" class="border-t border-divider px-4 py-3">
        <slot name="footer" />
      </div>
      <a
        v-if="props.href"
        :href="props.href"
        :aria-label="props.label ?? props.title"
        class="absolute -inset-px rounded-lg focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-indicator"
      ></a>
    </div>
  </component>
</template>
