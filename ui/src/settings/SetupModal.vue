<script setup lang="ts">
// First-run setup, one screen: main (the Claude app's account, where Warden installs its hooks), Warden's own sign-in
// for main's usage (now or later), the other account folders found, ticked, then Start. Skip, or Escape, installs nothing.
import { Check } from 'lucide-vue-next';
import { useElementSize, useScroll } from '@vueuse/core';
import { computed, ref, useTemplateRef, watch } from 'vue';
import { api, type Candidate } from '../api';
import AccountIcon from '../components/AccountIcon.vue';
import AppModal from '../components/AppModal.vue';
import { vLabelState } from '../components/labels';
import ProviderMark from '../components/ProviderMark.vue';
import WButton from '../components/WButton.vue';
import WTable from '../components/WTable.vue';
import WTableRow from '../components/WTableRow.vue';
import { act, store } from '../store';

const open = computed(() => store.state !== null && !store.state.setupDone);

// Whether the scrolling sections have more below what shows: then the divider above the buttons says so.
const body = useTemplateRef<HTMLElement>('body');
const { arrivedState } = useScroll(body);
const { height } = useElementSize(body);
const moreBelow = computed(() => {
  void height.value;
  const el = body.value;
  return !!el && el.scrollHeight > el.clientHeight + 1 && !arrivedState.bottom;
});

interface Row {
  c: Candidate;
  use: boolean;
}

const rows = ref<Row[]>([]);
const loading = ref(false);
const error = ref<string | null>(null);

async function load(): Promise<void> {
  loading.value = true;
  error.value = null;
  try {
    const found = (await api.candidates()).filter((c) => !c.known);
    rows.value = found.map((c) => ({ c, use: c.signedIn }));
  } catch {
    rows.value = [];
    error.value = 'Warden could not look for account folders.';
  }
  loading.value = false;
}

watch(open, (o) => o && void load(), { immediate: true });

const main = computed(() => store.state?.main ?? null);
const mainEmail = computed(() => main.value?.identity?.email ?? null);
/** The Claude app missing or signed out: setup says so and waits; the state updates by itself when it signs in. */
const appProblem = computed(() =>
  store.state?.problems.find((p) => p.kind === 'claudeAppMissing' || p.kind === 'claudeAppSignedOut'),
);
const signIn = computed(() => main.value?.signIn ?? null);

const startReason = computed(() =>
  loading.value
    ? 'Wait while Warden looks for account folders.'
    : appProblem.value
      ? 'Install and sign in to the Claude app first. You can skip setup for now.'
      : undefined,
);

async function finish(skip: boolean): Promise<void> {
  if (!skip && startReason.value) return;
  const accounts = skip
    ? []
    : rows.value.filter((r) => r.use).map((r) => ({ provider: r.c.provider, folder: r.c.folder, label: r.c.label }));
  error.value = await act(() => api.finishSetup(accounts, skip));
}

async function signInMain(): Promise<void> {
  error.value = await act(api.signInMain);
}

const CHECKBOX =
  'peer size-4.5 appearance-none rounded-sm bg-modal inset-ring-2 inset-ring-control hov:bg-chip-off press:bg-press checked:bg-primary checked:inset-ring-0 checked:hov:bg-primary-hover checked:press:bg-primary-press';

function identityText(c: Candidate): string {
  if (!c.signedIn) return 'Signed out';
  if (!c.identity) return c.folder;
  return c.identity.organization ? `${c.identity.email}, ${c.identity.organization}` : c.identity.email;
}
</script>

<template>
  <AppModal
    :open="open"
    title="Set up Warden"
    description="Main, its usage, and the other accounts Warden found."
    escape-only
    @update:open="(now) => now || finish(true)"
  >
    <!-- The title and the buttons stay in place; the sections between them scroll, and a divider above the buttons
         shows while there is more below. -->
    <form
      class="flex max-h-full w-full max-w-170 flex-col overflow-hidden rounded-2xl bg-modal shadow-modal"
      @submit.prevent="finish(false)"
    >
      <h2 class="shrink-0 px-7 pt-6 pb-3 text-18 font-semibold">Set up Warden</h2>

      <div
        ref="body"
        data-setup-body
        class="flex min-h-0 flex-auto flex-col gap-4 overflow-y-auto px-7 pb-4 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-indicator"
        :tabindex="moreBelow ? 0 : undefined"
        :aria-label="moreBelow ? 'Setup, scrolls' : undefined"
        :role="moreBelow ? 'region' : undefined"
      >
        <section aria-labelledby="setup-main" class="flex flex-col gap-2">
          <h3 id="setup-main" class="text-14 font-semibold">Main</h3>
          <div class="flex items-center gap-3">
            <AccountIcon v-if="main" :account="main" size="lg" on="modal" />
            <span class="flex min-w-0 flex-col gap-px">
              <span class="text-14 font-medium">The Claude app's account</span>
              <span v-if="appProblem" class="text-13 font-medium text-attn">{{
                appProblem.kind === 'claudeAppMissing'
                  ? 'The Claude app was not found. Install it and sign in.'
                  : 'The Claude app is signed out. Sign in to it.'
              }}</span>
              <span v-else class="truncate text-13 text-tertiary">{{ mainEmail }}</span>
            </span>
          </div>
          <p class="text-13 text-secondary">
            {{
              appProblem
                ? 'Setup carries on by itself once the Claude app is signed in.'
                : 'Warden installs its hooks in main when you press Start.'
            }}
          </p>
        </section>

        <section aria-labelledby="setup-usage" class="flex flex-col gap-2">
          <h3 id="setup-usage" class="text-14 font-semibold">Main's usage</h3>
          <p v-if="signIn?.state === 'signedIn'" class="flex items-center gap-1.5 text-13 text-secondary">
            <Check class="size-4 shrink-0 text-icon" aria-hidden="true" />Warden is signed in as {{ signIn.email }}.
          </p>
          <template v-else>
            <p v-if="signIn?.state === 'otherAccount'" class="text-13 font-medium text-attn">
              Warden is signed in as {{ signIn.email }}, not {{ mainEmail ?? "main's account" }}.
            </p>
            <p v-else-if="signIn?.state === 'expired'" class="text-13 font-medium text-attn">
              Warden's sign-in for main's usage has expired.
            </p>
            <div class="flex flex-wrap items-center gap-x-4 gap-y-2">
              <p class="min-w-0 flex-auto basis-80 text-13 text-secondary">
                Warden reads main's limits through a sign-in of its own, as {{ mainEmail ?? "main's account" }}. Until
                then, main counts as at its reserve. Or later, in Settings, Accounts.
              </p>
              <WButton @click="signInMain">{{ signIn?.state === 'signedOut' ? 'Sign in' : 'Sign in again' }}</WButton>
            </div>
          </template>
        </section>

        <section aria-labelledby="setup-others" class="flex flex-col gap-2">
          <h3 id="setup-others" class="text-14 font-semibold">Other accounts</h3>
          <p v-if="loading" class="text-14 text-tertiary">Looking for account folders…</p>

          <div v-else-if="rows.length === 0" class="rounded-lg border border-divider-strong p-4 text-14 text-tertiary">
            Warden found no other Claude or Codex account folders. Add them in Settings, Accounts.
          </div>

          <template v-else>
            <p class="text-13 text-secondary">Ticked folders take delegated work, with Warden's status hook in each.</p>
            <WTable aria-labelledby="setup-others" cols="grid-cols-setup gap-3">
              <template #head>
                <span role="columnheader"><span class="sr-only">Use</span></span>
                <span role="columnheader">Account</span>
              </template>
              <WTableRow v-for="(r, i) in rows" :key="r.c.folder" class="py-3">
                <span role="cell" class="relative flex size-5 items-center justify-center">
                  <input
                    :id="`setup-use-${i}`"
                    v-model="r.use"
                    v-label-state
                    type="checkbox"
                    :class="[
                      CHECKBOX,
                      'cursor-pointer focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-indicator',
                    ]"
                  />
                  <Check
                    class="pointer-events-none absolute size-3 text-on-primary opacity-0 peer-checked:opacity-100"
                    :stroke-width="3.2"
                    aria-hidden="true"
                  />
                </span>
                <div role="cell" class="flex min-w-0 items-center gap-3">
                  <span :class="['inline-flex size-7 shrink-0 items-center justify-center', !r.use && 'opacity-55']">
                    <ProviderMark
                      :provider="r.c.provider"
                      size="lg"
                      :label="r.c.provider === 'claude' ? 'Claude' : 'Codex'"
                    />
                  </span>
                  <span class="flex min-w-0 flex-col gap-px">
                    <label
                      :for="`setup-use-${i}`"
                      :class="['cursor-pointer text-14 font-semibold', !r.use && 'text-secondary']"
                      >{{ r.c.label }}</label
                    >
                    <span :class="['truncate text-13', r.c.signedIn ? 'text-tertiary' : 'font-medium text-attn']">
                      {{ identityText(r.c) }}
                    </span>
                  </span>
                </div>
              </WTableRow>
            </WTable>
            <span class="text-13 text-tertiary"
              >Rename them, or add other folders and new accounts, later in Settings, Accounts.</span
            >
          </template>
        </section>

        <p v-if="error" role="alert" class="text-13 font-medium text-attn">{{ error }}</p>
      </div>

      <div
        :class="[
          'flex shrink-0 justify-end gap-2 border-t px-7 py-3',
          moreBelow ? 'border-divider' : 'border-transparent',
        ]"
        data-setup-buttons
      >
        <WButton variant="primary" type="submit" :disabled-reason="startReason">Start</WButton>
        <WButton @click="finish(true)">Skip for now</WButton>
      </div>
    </form>
  </AppModal>
</template>
