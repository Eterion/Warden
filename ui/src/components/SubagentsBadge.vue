<script setup lang="ts">
// A session's subagents: the robot and their count. Hovering or focusing it lists their types.
import { Bot } from '@lucide/vue';
import { TooltipContent, TooltipPortal, TooltipProvider, TooltipRoot, TooltipTrigger } from 'reka-ui';

const props = defineProps<{ label: string; count: number }>();
</script>

<template>
  <TooltipProvider :delay-duration="200">
    <TooltipRoot>
      <TooltipTrigger
        :aria-label="props.label"
        class="inline-flex cursor-default items-center gap-1.5 rounded-sm text-tertiary focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-indicator"
      >
        <Bot class="size-3.5 text-icon" aria-hidden="true" />{{ props.count }}
      </TooltipTrigger>
      <TooltipPortal>
        <TooltipContent
          side="top"
          :side-offset="6"
          class="z-50 rounded-lg bg-primary px-2 py-1 text-12 text-on-primary shadow-modal"
        >
          {{ props.label }}
        </TooltipContent>
      </TooltipPortal>
    </TooltipRoot>
  </TooltipProvider>
</template>
