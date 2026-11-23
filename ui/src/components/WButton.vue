<script setup lang="ts">
// A button with text. Primary comes first in a group; outline is the everyday one (sm: small, on a list row); quiet
// sits among chips (md) or, small as the legends' text (sm), on the window background or, `on="panel"`, on the
// content panel (the Limit resets filter), lighting with the panel's hover; link is underlined text. An icon, when
// given, comes before the text; busy sets aria-busy and turns the icon slowly. One that cannot be used says why
// (`disabledReason`); one usable only sometimes passes `null` while it is, so it stays the same element and keeps focus.
import type { Component } from 'vue';
import { vSyncSpin } from './spin';
import WTooltip from './WTooltip.vue';

defineOptions({ inheritAttrs: false });
const emit = defineEmits<{ click: [event: MouseEvent] }>();

const props = withDefaults(
  defineProps<{
    variant?: 'primary' | 'outline' | 'quiet' | 'link';
    size?: 'md' | 'sm';
    icon?: Component;
    busy?: boolean;
    type?: 'button' | 'submit';
    disabledReason?: string | null;
    /** What a small quiet button sits on: the window background (default) or the content panel. */
    on?: 'win' | 'panel';
  }>(),
  {
    variant: 'outline',
    size: 'md',
    icon: undefined,
    busy: false,
    type: 'button',
    disabledReason: undefined,
    on: 'win',
  },
);

const BASE =
  'inline-flex shrink-0 cursor-pointer items-center focus-visible:outline-2 focus-visible:outline-indicator disabled:cursor-not-allowed disabled:opacity-40';

function look(): string {
  const icon = !!props.icon;
  switch (props.variant) {
    case 'primary':
      return `h-9 rounded-lg bg-primary text-14 font-semibold text-on-primary shadow-button hov:bg-primary-hover press:bg-primary-press focus-visible:outline-offset-2 ${icon ? 'gap-1.5 pr-3.5 pl-2.75' : 'px-3.5'}`;
    case 'outline':
      if (props.size === 'sm')
        return 'h-7 rounded-md px-2.5 text-12 font-semibold text-text inset-ring-1 inset-ring-outline-border hov:bg-chip-off press:bg-press focus-visible:outline-offset-2';
      return `h-9 rounded-lg text-14 font-semibold text-text inset-ring-1 inset-ring-outline-border hov:bg-chip-off press:bg-press focus-visible:outline-offset-2 ${icon ? 'gap-2 pr-3.5 pl-3' : 'px-3.5'}`;
    case 'quiet':
      return props.size === 'md'
        ? 'h-8 gap-1.5 rounded-lg pr-3 pl-2.5 text-14 font-medium text-secondary hov:bg-chip-off press:bg-press focus-visible:outline-offset-2'
        : `h-7 gap-1.5 rounded-md px-2 text-12 text-secondary press:bg-press ${props.on === 'panel' ? 'hov:bg-chip-off' : 'hov:bg-active'}`;
    case 'link':
      return 'h-6 rounded-sm px-1.5 text-12 text-secondary underline underline-offset-2 hov:text-text press:bg-press press:text-text';
  }
}

function iconClass(): string {
  if (props.variant === 'primary') return 'size-4';
  if (props.variant === 'quiet') return props.size === 'md' ? 'size-3.5' : 'size-3 text-icon';
  return 'size-4 text-icon';
}
</script>

<template>
  <!-- One element whether or not it can be used now (`null`: it can), so focus stays on it as that changes. -->
  <WTooltip
    v-if="props.disabledReason !== undefined"
    :text="props.disabledReason ?? ''"
    :disabled="!props.disabledReason"
    as-child
  >
    <button
      v-bind="$attrs"
      :type="props.disabledReason ? 'button' : props.type"
      :aria-disabled="props.disabledReason ? 'true' : undefined"
      :class="[BASE, look(), props.disabledReason && 'cursor-not-allowed']"
      @click="!props.disabledReason && emit('click', $event)"
    >
      <span :class="['inline-flex items-center gap-1.5', props.disabledReason && 'opacity-40']">
        <component :is="props.icon" v-if="props.icon" :class="iconClass()" aria-hidden="true" />
        <slot />
      </span>
    </button>
  </WTooltip>
  <button
    v-else
    v-bind="$attrs"
    :type="props.type"
    :aria-busy="props.busy || undefined"
    :class="[BASE, look()]"
    @click="emit('click', $event)"
  >
    <component
      :is="props.icon"
      v-if="props.icon"
      v-sync-spin
      :class="[iconClass(), props.busy && 'animate-spin-slow']"
      :stroke-width="props.variant === 'primary' ? 2.2 : undefined"
      aria-hidden="true"
    />
    <slot />
  </button>
</template>
