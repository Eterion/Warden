<script setup lang="ts">
// The confirmation before a limit reset is used: what it does and that it cannot be undone, then "Use reset" and
// "Cancel". While it is sent, "Use reset" shows busy and the modal stays open (neither Escape nor a click beside it
// closes it); the server's refusal shows in the modal and the reset stays; once it is used, the modal closes and its row
// goes. Sending is the caller's (`send`, answering an error sentence or null), so the row's exit can follow.
import { RotateCw } from 'lucide-vue-next';
import { ref, watch } from 'vue';
import type { ResetRow } from '../format';
import { resetEffect } from '../format';
import AppModal from './AppModal.vue';
import { keptWhileClosing } from './kept';
import WButton from './WButton.vue';

const open = defineModel<boolean>('open', { required: true });
const props = defineProps<{
  reset: ResetRow | null;
  send: () => Promise<string | null>;
  /** Where focus goes as it closes, when it gives an element (after a use, the opener's row is going). */
  returnTo?: () => HTMLElement | null;
}>();

/** What the modal draws: the closing animation still shows the reset it was opened for. */
const shown = keptWhileClosing(
  () => open.value,
  () => props.reset,
);
const busy = ref(false);
const error = ref<string | null>(null);

watch(open, (o) => {
  if (o) error.value = null;
});

/** "Full reset", or "reset" for one without a name of its own. */
function name(r: ResetRow | null): string {
  return r && r.title !== 'Reset' ? r.title : 'reset';
}

async function confirm(): Promise<void> {
  if (busy.value) return;
  busy.value = true;
  error.value = null;
  const failed = await props.send();
  busy.value = false;
  if (failed) error.value = failed;
  else open.value = false;
}
</script>

<template>
  <AppModal
    v-model:open="open"
    :title="`Use a ${name(shown)} on ${shown?.account.label ?? ''}?`"
    :blocking="busy"
    :return-to="props.returnTo"
  >
    <form class="flex w-full max-w-120 flex-col gap-5 rounded-2xl bg-modal p-7 shadow-modal" @submit.prevent="confirm">
      <div class="flex flex-col gap-1">
        <h2 class="text-18 font-semibold">Use a {{ name(shown) }} on {{ shown?.account.label }}?</h2>
        <span class="text-14 text-tertiary">{{ resetEffect(shown?.title ?? '') }} This can't be undone.</span>
      </div>
      <p v-if="error" role="alert" class="text-13 font-medium text-attn" data-use-reset-error>{{ error }}</p>
      <div class="flex justify-end gap-2">
        <WButton variant="primary" type="submit" :icon="RotateCw" :busy="busy">Use reset</WButton>
        <WButton :disabled="busy" @click="open = false">Cancel</WButton>
      </div>
    </form>
  </AppModal>
</template>
