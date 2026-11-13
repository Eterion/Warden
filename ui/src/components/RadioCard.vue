<script setup lang="ts">
// A radio card inside a Reka RadioGroupRoot: a bordered card with an icon at the start (a provider mark, or a folder),
// a title, a line of text and a round check, ringed when chosen.
//
// A card that cannot be chosen says why (`disabledReason`): it stays focusable (and in the arrow-key order) with
// `aria-disabled`, shows the reason in the shared tooltip on hover and keyboard focus, and choosing it does nothing.
// Only its content fades, never its focus ring, and it has no hover or pressed state.
import { Check } from 'lucide-vue-next';
import { createReusableTemplate } from '@vueuse/core';
import { RadioGroupItem } from 'reka-ui';
import { computed } from 'vue';
import WTooltip from './WTooltip.vue';

const props = withDefaults(
  defineProps<{
    value: string;
    title: string;
    text: string;
    disabledReason?: string;
  }>(),
  { disabledReason: undefined },
);

const locked = computed(() => props.disabledReason !== undefined);

const MARK =
  'inline-flex size-4.5 shrink-0 items-center justify-center rounded-full text-transparent inset-ring-2 inset-ring-control group-data-[state=checked]:bg-primary group-data-[state=checked]:text-on-primary group-data-[state=checked]:inset-ring-0';
const CARD =
  'group flex rounded-lg bg-modal text-left inset-ring-1 inset-ring-ring-strong focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-indicator';
const LIVE =
  'cursor-pointer hov:inset-ring-control press:bg-row-press data-[state=checked]:inset-ring-2 data-[state=checked]:inset-ring-indicator data-[state=checked]:hov:bg-row-hover data-[state=checked]:press:bg-row-press';

const cardClass = computed(() => [CARD, locked.value ? 'cursor-not-allowed' : LIVE, 'items-center gap-3 px-3.5 py-3']);
// The tooltip trigger sets its own `data-state`, so only a locked card (never checked) sits inside one.
const [DefineBody, Body] = createReusableTemplate();
const fade = computed(() => (locked.value ? 'opacity-40' : undefined));

/** A locked card is never chosen: Reka checks an item on click, Space and arrow keys unless its select is cancelled. */
function onSelect(event: Event): void {
  if (locked.value) event.preventDefault();
}
</script>

<template>
  <DefineBody>
    <span :class="['inline-flex size-7 shrink-0 items-center justify-center', fade]"><slot name="icon" /></span>
    <span :class="['flex flex-auto flex-col gap-px', fade]">
      <span class="text-14 font-semibold">{{ props.title }}</span>
      <span class="text-12 text-tertiary">{{ props.text }}</span>
    </span>
    <span aria-hidden="true" :class="[MARK, fade]"><Check class="size-2.75" :stroke-width="3.2" /></span>
  </DefineBody>
  <WTooltip v-if="props.disabledReason" :text="props.disabledReason" as-child>
    <RadioGroupItem :value="props.value" aria-disabled="true" :class="cardClass" @select="onSelect">
      <Body />
    </RadioGroupItem>
  </WTooltip>
  <RadioGroupItem v-else :value="props.value" :class="cardClass">
    <Body />
  </RadioGroupItem>
</template>
