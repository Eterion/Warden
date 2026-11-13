<script setup lang="ts">
// An item in the sidebar or the Settings nav: an icon and a label, filled and bold when it is the current page. With
// `href` it is a link, otherwise a button; clicks fall through to it.
import type { Component } from 'vue';

const props = defineProps<{ icon: Component; current?: boolean; href?: string }>();
</script>

<template>
  <component
    :is="props.href === undefined ? 'button' : 'a'"
    :type="props.href === undefined ? 'button' : undefined"
    :href="props.href"
    :aria-current="props.current ? 'page' : undefined"
    :class="[
      'flex min-h-9 w-full cursor-pointer items-center gap-3 rounded-lg px-2 text-left text-14 text-text focus-visible:outline-2 focus-visible:outline-indicator',
      props.current
        ? 'bg-active font-semibold shadow-active hov:bg-active-hover press:bg-press'
        : 'font-medium hov:bg-active press:bg-press',
    ]"
  >
    <component
      :is="props.icon"
      :class="['size-4.5 shrink-0', props.current ? 'text-text' : 'text-icon']"
      aria-hidden="true"
    />
    <slot />
  </component>
</template>
