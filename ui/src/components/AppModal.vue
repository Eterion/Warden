<script setup lang="ts">
// A modal on Reka UI's dialog: focus stays inside, Escape or a click beside it closes it (`escapeOnly`: only Escape;
// `blocking`: neither, while something must finish). A second modal stacks on
// top of the first, over a lighter overlay. The slot draws the card itself. It opens with a soft scale and a mild
// overshoot while the backdrop fades in, and closes quicker; with reduced motion both only fade.
import { DialogContent, DialogDescription, DialogOverlay, DialogPortal, DialogRoot, DialogTitle } from 'reka-ui';
import { watch } from 'vue';

const open = defineModel<boolean>('open', { required: true });
const props = withDefaults(
  defineProps<{
    title: string;
    description?: string;
    stacked?: boolean;
    escapeOnly?: boolean;
    blocking?: boolean;
    /** Where focus goes as it closes, in place of whatever opened it, when this gives an element. */
    returnTo?: () => HTMLElement | null;
  }>(),
  { description: undefined, stacked: false, escapeOnly: false, blocking: false, returnTo: undefined },
);

/**
 * Focus goes back to whatever opened the modal when it closes. The modals open from state rather than from a
 * Reka trigger, so the element that had focus is remembered as the modal opens.
 */
let opener: HTMLElement | null = null;
watch(
  open,
  (now) => {
    if (now && document.activeElement instanceof HTMLElement) opener = document.activeElement;
  },
  { immediate: true, flush: 'sync' },
);

function returnFocus(e: Event): void {
  const to = props.returnTo?.() ?? (opener?.isConnected ? opener : null);
  if (to) {
    e.preventDefault();
    to.focus();
  }
  opener = null;
}

/** A click on the space around the card closes the modal, as a click on the overlay would (unless `escapeOnly`). */
function onBackdrop(e: MouseEvent): void {
  if (!props.escapeOnly && !props.blocking && e.target === e.currentTarget) open.value = false;
}

/** With `escapeOnly` or `blocking`, Reka's own close on a pointer outside the content is refused too. */
function onOutside(e: Event): void {
  if (props.escapeOnly || props.blocking) e.preventDefault();
}

/** A blocking modal ignores Escape. */
function onEscape(e: KeyboardEvent): void {
  if (props.blocking) e.preventDefault();
}
</script>

<template>
  <DialogRoot v-model:open="open">
    <DialogPortal>
      <DialogOverlay
        :class="[
          'fixed inset-0 data-[state=closed]:animate-fade-out data-[state=open]:animate-fade-in',
          props.stacked ? 'z-30 bg-overlay2' : 'z-10 bg-overlay',
        ]"
      />
      <DialogContent
        :class="[
          'fixed inset-0 flex items-center justify-center p-8 text-text outline-none data-[state=closed]:animate-modal-out data-[state=open]:animate-modal-in',
          props.stacked ? 'z-40' : 'z-20',
        ]"
        @click="onBackdrop"
        @pointer-down-outside="onOutside"
        @escape-key-down="onEscape"
        @close-auto-focus="returnFocus"
      >
        <DialogTitle class="sr-only">{{ props.title }}</DialogTitle>
        <DialogDescription v-if="props.description" class="sr-only">{{ props.description }}</DialogDescription>
        <slot />
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>
