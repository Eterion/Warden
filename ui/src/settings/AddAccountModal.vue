<script setup lang="ts">
// Add a new account through its official sign-in, or an existing Warden folder.
import { FolderOpen } from 'lucide-vue-next';
import { RadioGroupRoot } from 'reka-ui';
import { computed, ref, watch } from 'vue';
import { api, type Candidate, type Provider } from '../api';
import AppModal from '../components/AppModal.vue';
import ProviderMark from '../components/ProviderMark.vue';
import RadioCard from '../components/RadioCard.vue';
import SegmentedControl from '../components/SegmentedControl.vue';
import ToggleChip from '../components/ToggleChip.vue';
import WButton from '../components/WButton.vue';
import WTextField from '../components/WTextField.vue';
import { desktop, pickFolder } from '../desktop';
import { newFolderPreview } from '../format';
import { act, store } from '../store';

const open = defineModel<boolean>('open', { required: true });
/** Over Settings (opened from Settings › Accounts) rather than on its own (from the sidebar). */
const props = defineProps<{ stacked?: boolean }>();

type Mode = 'new' | 'existing';

const provider = ref<Provider>('claude');
const mode = ref<Mode>('new');
const name = ref('');
const folder = ref('');
const error = ref<string | null>(null);
/** An account is being added: true until its answer comes, across openings, so a second one is never sent meanwhile. */
const busy = ref(false);
const found = ref<Candidate[]>([]);
const providerName = computed(() => (provider.value === 'claude' ? 'Claude' : 'Codex'));

function defaultName(): string {
  const s = store.state;
  const n =
    (s?.accounts.filter((a) => a.provider === provider.value).length ?? 0) +
    (s?.main?.provider === provider.value ? 1 : 0) +
    1;
  return `${providerName.value} ${n}`;
}

/** Counts openings, so a request started in an earlier one never closes a later one or shows its refusal there. */
let opening = 0;

watch(
  open,
  (o) => {
    if (!o) return;
    opening++;
    provider.value = 'claude';
    mode.value = 'new';
    folder.value = '';
    error.value = null;
    name.value = defaultName();
    found.value = [];
    api.candidates().then(
      (c) => (found.value = c),
      () => (found.value = []),
    );
  },
  { immediate: true },
);

watch(provider, () => {
  name.value = defaultName();
  folder.value = '';
  error.value = null;
});

watch(mode, () => (error.value = null));

const preview = computed(() => newFolderPreview(store.state?.homeDir ?? null, provider.value, name.value));
/** Signed-in Warden folders to pick from; desktop app folders are excluded. */
const suggestions = computed(() => found.value.filter((c) => c.provider === provider.value && !c.known && c.signedIn));

function pickFound(c: Candidate): void {
  folder.value = c.folder;
  name.value = c.label;
}

async function choose(): Promise<void> {
  const picked = await pickFolder().catch(() => null);
  if (picked) folder.value = picked;
}

async function submit(): Promise<void> {
  if (busy.value) return;
  busy.value = true;
  const mine = opening;
  const failed = await act(() =>
    mode.value === 'new'
      ? api.addNew(provider.value, name.value)
      : api.addExisting(provider.value, folder.value, name.value),
  );
  busy.value = false;
  // Closed (and maybe opened again) while it was sent: this opening is not the one it came from.
  if (mine !== opening || !open.value) return;
  error.value = failed;
  if (!failed) open.value = false;
}

const providers = [
  { id: 'claude', label: 'Claude', text: 'Anthropic' },
  { id: 'codex', label: 'Codex', text: 'OpenAI' },
] as const;
</script>

<template>
  <AppModal v-model:open="open" title="Add account" :stacked="props.stacked">
    <form
      class="flex max-h-full w-full max-w-115 flex-col gap-5 overflow-y-auto rounded-2xl bg-modal p-7 shadow-modal"
      @submit.prevent="submit"
    >
      <div class="flex flex-col gap-1">
        <h2 class="text-18 font-semibold">Add account</h2>
      </div>

      <RadioGroupRoot v-model="provider" aria-label="Provider" orientation="horizontal" class="grid grid-cols-2 gap-3">
        <RadioCard v-for="p in providers" :key="p.id" :value="p.id" :title="p.label" :text="p.text">
          <template #icon><ProviderMark :provider="p.id" size="lg" /></template>
        </RadioCard>
      </RadioGroupRoot>

      <SegmentedControl
        v-model="mode"
        label="How to add"
        :options="[
          { value: 'new', label: 'New account' },
          { value: 'existing', label: 'Existing folder' },
        ]"
        full
      />

      <div class="flex flex-col border-t border-divider-strong">
        <div class="flex items-center gap-3 border-b border-divider py-3">
          <label for="add-name" class="w-16 shrink-0 text-14 text-tertiary">Name</label>
          <WTextField id="add-name" v-model="name" maxlength="60" class="min-w-0 flex-auto" />
        </div>
        <div class="flex items-center gap-3 border-b border-divider py-3">
          <template v-if="mode === 'existing'">
            <label for="add-folder" class="w-16 shrink-0 text-14 text-tertiary">Folder</label>
            <div class="flex min-w-0 flex-auto gap-2">
              <WTextField
                id="add-folder"
                v-model="folder"
                kind="path"
                :placeholder="`Path of a ${providerName} folder`"
                class="min-w-0 flex-auto"
              />
              <WButton v-if="desktop.inApp" :icon="FolderOpen" @click="choose">Choose folder…</WButton>
            </div>
          </template>
          <template v-else>
            <span class="w-16 shrink-0 text-14 text-tertiary">Folder</span>
            <span class="min-h-9 min-w-0 flex-auto content-center font-mono text-12 wrap-anywhere text-secondary">{{
              preview
            }}</span>
          </template>
        </div>
        <div v-if="mode === 'existing' && suggestions.length" class="flex flex-col gap-2 pt-3">
          <span id="found-label" class="text-13 text-tertiary">Signed-in folders Warden found</span>
          <div role="group" aria-labelledby="found-label" class="flex flex-wrap gap-2">
            <ToggleChip
              v-for="c in suggestions"
              :key="c.folder"
              look="outline"
              :pressed="folder === c.folder"
              :title="c.identity?.email"
              class="font-mono"
              @click="pickFound(c)"
            >
              {{ c.folder }}
            </ToggleChip>
          </div>
        </div>
      </div>

      <p v-if="error" role="alert" class="text-13 font-medium text-attn">{{ error }}</p>

      <div class="flex justify-end gap-2">
        <WButton variant="primary" type="submit" :busy="busy">
          {{ mode === 'new' ? 'Create and sign in' : 'Add folder' }}
        </WButton>
        <WButton @click="open = false">Cancel</WButton>
      </div>
    </form>
  </AppModal>
</template>
