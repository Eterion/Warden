<script setup lang="ts">
// The sidebar's update, above Settings (desktop app): "Downloading update" with its version and progress while a newer
// Warden downloads, then one "Install and restart" button. It asks first; on yes the same modal blocks the app while
// the update installs, until Warden closes for the installer and starts again. If the installer cannot start, the
// modal says why and asks again. Nothing while there is no update.
import { CircleFadingArrowUp } from '@lucide/vue';
import { computed, nextTick, ref } from 'vue';
import { ApiError, api } from '../api';
import AppModal from '../components/AppModal.vue';
import StateIcon from '../components/StateIcon.vue';
import WButton from '../components/WButton.vue';
import { apply, store } from '../store';

const ASK = 'Warden closes, installs the update and starts again.';
const WAIT = 'Warden starts again when it is done.';

const update = computed(() => store.state?.update ?? null);
const phase = ref<'closed' | 'confirm' | 'installing'>('closed');
const error = ref<string | null>(null);
const form = ref<HTMLFormElement | null>(null);
const installing = ref<HTMLElement | null>(null);

/** The modal is open while asking or installing; only asking can be closed. */
const open = computed({
  get: () => phase.value !== 'closed',
  set: (v) => {
    if (!v && phase.value === 'confirm') phase.value = 'closed';
  },
});

function ask(): void {
  error.value = null;
  phase.value = 'confirm';
}

async function install(): Promise<void> {
  error.value = null;
  phase.value = 'installing';
  // Focus moves to what replaced the form, so it stays inside the modal.
  await nextTick();
  installing.value?.focus();
  try {
    apply(await api.installUpdate());
  } catch (e) {
    // Warden closes inside the request when the installer starts, so its answer never comes. An answer with an error
    // means it could not start: say why and ask again.
    if (e instanceof ApiError) {
      error.value = e.message;
      phase.value = 'confirm';
      await nextTick();
      form.value?.querySelector<HTMLButtonElement>('button[type=submit]')?.focus();
    }
  }
}
</script>

<template>
  <div v-if="update" class="flex flex-col gap-2 px-2 pb-2">
    <template v-if="!update.ready">
      <div class="flex items-baseline justify-between gap-2 text-13">
        <span class="font-medium">Downloading update</span>
        <span class="text-secondary">{{ update.version }}</span>
      </div>
      <div
        role="progressbar"
        :aria-label="`Downloading Warden ${update.version}`"
        aria-valuemin="0"
        aria-valuemax="100"
        :aria-valuenow="update.downloaded ?? undefined"
        class="h-1 overflow-hidden rounded-full bg-track-win"
      >
        <span class="block h-full rounded-full bg-fill" :style="{ width: `${update.downloaded ?? 0}%` }"></span>
      </div>
    </template>
    <WButton
      v-else
      :icon="CircleFadingArrowUp"
      :aria-label="`Install and restart, Warden ${update.version}`"
      class="justify-center"
      @click="ask"
      >Install and restart</WButton
    >

    <AppModal
      v-model:open="open"
      :title="phase === 'installing' ? `Installing Warden ${update.version}` : `Install Warden ${update.version}?`"
      :description="phase === 'installing' ? WAIT : ASK"
      :blocking="phase === 'installing'"
    >
      <form
        v-if="phase !== 'installing'"
        ref="form"
        class="flex w-full max-w-120 flex-col gap-5 rounded-2xl bg-modal p-7 shadow-modal"
        @submit.prevent="install"
      >
        <div class="flex flex-col gap-1">
          <h2 class="text-18 font-semibold">Install Warden {{ update.version }}?</h2>
          <span class="text-14 text-tertiary">{{ ASK }}</span>
        </div>
        <p v-if="error" role="alert" class="text-13 font-medium text-attn">{{ error }}</p>
        <div class="flex justify-end gap-2">
          <WButton variant="primary" type="submit">Install and restart</WButton>
          <WButton @click="phase = 'closed'">Cancel</WButton>
        </div>
      </form>
      <div
        v-else
        ref="installing"
        role="status"
        tabindex="-1"
        class="flex w-full max-w-120 flex-col gap-1 rounded-2xl bg-modal p-7 shadow-modal outline-none"
      >
        <h2 class="flex items-center gap-2.5 text-18 font-semibold">
          Installing Warden {{ update.version }}
          <StateIcon state="working" />
        </h2>
        <span class="text-14 text-tertiary">{{ WAIT }}</span>
      </div>
    </AppModal>
  </div>
</template>
