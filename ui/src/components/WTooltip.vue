<script setup lang="ts">
// A short explanation shown on hover and on keyboard focus. The slot is the trigger: drawn as a plain button so it
// can take focus, styled by the caller through its classes; or, with `as-child`, the caller's own focusable control.
//
// The explanation is drawn into the modal the trigger sits in, if any, so it stays inside that dialog's landmark.
import { TooltipContent, TooltipPortal, TooltipProvider, TooltipRoot, TooltipTrigger } from 'reka-ui';
import { useTemplateRef } from 'vue';
import { usePortalTarget } from './portal';

defineOptions({ inheritAttrs: false });
const props = defineProps<{ text: string; asChild?: boolean }>();

const trigger = useTemplateRef<{ $el: Node | null }>('trigger');
const to = usePortalTarget(trigger);
</script>

<template>
  <TooltipProvider :delay-duration="300">
    <TooltipRoot>
      <TooltipTrigger v-if="props.asChild" ref="trigger" v-bind="$attrs" as-child>
        <slot />
      </TooltipTrigger>
      <TooltipTrigger
        v-else
        ref="trigger"
        v-bind="$attrs"
        type="button"
        class="cursor-default rounded-sm hov:bg-selected-hover press:bg-selected-press focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-indicator"
      >
        <slot />
      </TooltipTrigger>
      <TooltipPortal :to="to">
        <TooltipContent
          :side-offset="6"
          class="z-50 max-w-80 rounded-md bg-primary px-2.5 py-1.5 text-12 text-on-primary shadow-modal"
        >
          {{ props.text }}
        </TooltipContent>
      </TooltipPortal>
    </TooltipRoot>
  </TooltipProvider>
</template>
