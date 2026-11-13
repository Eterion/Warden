<script setup lang="ts">
// Renaming or removing an account, as a second modal on top of Settings.
import { ref, watch } from 'vue';
import { api, type Account } from '../api';
import AppModal from '../components/AppModal.vue';
import WButton from '../components/WButton.vue';
import WTextField from '../components/WTextField.vue';
import { act } from '../store';

const open = defineModel<boolean>('open', { required: true });
const props = defineProps<{ account: Account | null; mode: 'rename' | 'remove' }>();
const name = ref('');
const error = ref<string | null>(null);

watch(open, (o) => {
  if (o) {
    name.value = props.account?.label ?? '';
    error.value = null;
  }
});

async function confirm(): Promise<void> {
  const a = props.account;
  if (!a) return;
  error.value = await act(() =>
    props.mode === 'rename'
      ? a.id === 'main'
        ? api.renameMain(name.value)
        : api.update(a.id, { label: name.value })
      : api.remove(a.id),
  );
  if (!error.value) open.value = false;
}
</script>

<template>
  <AppModal
    v-model:open="open"
    :title="props.mode === 'rename' ? `Rename ${props.account?.label ?? ''}` : `Remove ${props.account?.label ?? ''}`"
    stacked
  >
    <form class="flex w-full max-w-120 flex-col gap-5 rounded-2xl bg-modal p-7 shadow-modal" @submit.prevent="confirm">
      <div class="flex flex-col gap-1">
        <h2 class="text-18 font-semibold">
          {{ props.mode === 'rename' ? 'Rename account' : `Remove ${props.account?.label}?` }}
        </h2>
        <span v-if="props.mode === 'remove'" class="text-14 text-tertiary">
          Warden forgets the account and its readings. The folder and its login stay as they are; add it again any time.
        </span>
      </div>
      <div class="flex flex-col gap-3">
        <div v-if="props.mode === 'rename'" class="flex items-center gap-3">
          <label for="rename-name" class="w-16 shrink-0 text-14 text-tertiary">Name</label>
          <WTextField id="rename-name" v-model="name" maxlength="60" class="min-w-0 flex-auto" />
        </div>
        <!-- The folder, which the Accounts table shows only on hover. -->
        <div class="flex items-center gap-3">
          <span class="w-16 shrink-0 text-14 text-tertiary">Folder</span>
          <span class="min-w-0 flex-auto font-mono text-12 wrap-anywhere text-secondary">{{
            props.account?.folder
          }}</span>
        </div>
      </div>
      <p v-if="error" role="alert" class="text-13 font-medium text-attn">{{ error }}</p>
      <div class="flex justify-end gap-2">
        <WButton variant="primary" type="submit">{{ props.mode === 'rename' ? 'Save' : 'Remove' }}</WButton>
        <WButton @click="open = false">Cancel</WButton>
      </div>
    </form>
  </AppModal>
</template>
