<script setup lang="ts">
// The Settings modal: its own sidebar with the sections (General and Accounts) and a search box.
import { ChevronRight, CircleUserRound, Plus, Settings, X } from 'lucide-vue-next';
import { computed, ref, watch } from 'vue';
import AppModal from '../components/AppModal.vue';
import NavItem from '../components/NavItem.vue';
import WButton from '../components/WButton.vue';
import WList from '../components/WList.vue';
import WListItem from '../components/WListItem.vue';
import WIconButton from '../components/WIconButton.vue';
import WTextField from '../components/WTextField.vue';
import { closeModal, go, route, type Section } from '../route';
import { keptWhileClosing } from '../components/kept';
import { store } from '../store';
import AccountsSection from './AccountsSection.vue';
import AddAccountModal from './AddAccountModal.vue';
import GeneralSection from './GeneralSection.vue';

const open = computed({
  get: () => route.settingsOpen.value,
  set: (v: boolean) => {
    if (!v) closeModal();
  },
});
const addOpen = computed({
  get: () => route.addOpen.value,
  set: (v: boolean) => go(v ? 'settings/accounts/add' : 'settings/accounts'),
});

const query = ref('');
// Cleared when Settings opens again, not as it closes, so the closing modal keeps what it showed.
watch(open, (o) => {
  if (o) query.value = '';
});
/** The section drawn: closing clears the address, which would otherwise show General while the modal closes. */
const section = keptWhileClosing(
  () => open.value,
  () => route.section.value,
);
const searching = computed(() => query.value.trim().length > 0);

const sections: { id: Section; label: string; icon: typeof Settings; title: string; subtitle: string }[] = [
  {
    id: 'general',
    label: 'General',
    icon: Settings,
    title: 'General',
    subtitle: 'Usage reading and the theme.',
  },
  {
    id: 'accounts',
    label: 'Accounts',
    icon: CircleUserRound,
    title: 'Accounts',
    subtitle: 'Main and the accounts that take delegated work.',
  },
];

const INDEX: {
  s: Section;
  label: string;
  help: string;
  keys: string;
  add?: boolean;
  desktop?: boolean;
}[] = [
  {
    s: 'accounts',
    label: 'Add account',
    help: 'A new Claude or Codex account, or an existing folder.',
    keys: 'new login folder',
    add: true,
  },
  {
    s: 'accounts',
    label: 'Sign in',
    help: "Warden's sign-in for main's usage, or an account signed out.",
    keys: 'login sign in usage expired',
  },
  {
    s: 'accounts',
    label: 'Keep for my own work',
    help: "How much of main's week it keeps before handing work to the other accounts.",
    keys: 'reserve main delegate delegated work limit',
  },
  {
    s: 'accounts',
    label: 'My work hours',
    help: "The days and hours you usually work: main's reserve shrinks only during them.",
    keys: 'work hours days time schedule reserve weekend evening',
  },
  {
    s: 'accounts',
    label: 'Use an account',
    help: 'Switch an account off so Warden knows it but never sends it delegated work.',
    keys: 'enable disable off personal exclude delegated work hooks',
  },
  {
    s: 'general',
    label: 'Theme',
    help: 'System, light or dark.',
    keys: 'dark light appearance',
  },
  {
    s: 'general',
    label: 'Read usage every',
    help: 'How often account limits are read.',
    keys: 'usage refresh interval limit',
  },
  {
    s: 'general',
    label: 'Keep running in the tray',
    help: 'Closing the window quits Warden unless this is on.',
    keys: 'close quit background tray notification area',
    desktop: true,
  },
  {
    s: 'general',
    label: 'Start with Windows',
    help: 'Open in the background when you sign in.',
    keys: 'startup autostart login',
    desktop: true,
  },
  {
    s: 'general',
    label: 'Run setup again',
    help: 'First-run setup, with the account folders Warden finds.',
    keys: 'setup first run data folder',
  },
  {
    s: 'general',
    label: 'Open log folder',
    help: "Warden's own log files, in Explorer.",
    keys: 'logs log files explorer',
    desktop: true,
  },
];

const results = computed(() => {
  const q = query.value.trim().toLowerCase();
  const inApp = store.state?.desktop != null;
  return INDEX.filter((i) => (inApp || !i.desktop) && `${i.label} ${i.help} ${i.keys}`.toLowerCase().includes(q));
});

const current = computed(() => sections.find((s) => s.id === section.value) ?? sections[0]!);
const title = computed(() => (searching.value ? 'Search' : current.value.title));
const subtitle = computed(() => {
  if (!searching.value) return current.value.subtitle;
  const n = results.value.length;
  return `${n} ${n === 1 ? 'setting matches' : 'settings match'} "${query.value.trim()}"`;
});

function pick(s: Section): void {
  query.value = '';
  go(`settings/${s}`);
}

function goTo(r: (typeof INDEX)[number]): void {
  query.value = '';
  go(r.add ? 'settings/accounts/add' : `settings/${r.s}`);
}
</script>

<template>
  <AppModal v-model:open="open" title="Settings">
    <div class="flex h-205 max-h-full w-full max-w-240 flex-wrap overflow-hidden rounded-2xl bg-modal shadow-modal">
      <nav aria-label="Settings sections" class="flex max-w-55 grow basis-47.5 flex-col gap-0.5 bg-nav px-3 py-4">
        <WTextField
          v-model="query"
          kind="search"
          label="Search settings"
          placeholder="Search settings"
          class="mb-3.5"
        />
        <NavItem
          v-for="s in sections"
          :key="s.id"
          :icon="s.icon"
          :current="!searching && s.id === section"
          @click="pick(s.id)"
        >
          {{ s.label }}
        </NavItem>
        <span v-if="store.state?.hooksOff" class="mt-auto px-2 text-12 text-secondary">Hooks are off for this run</span>
        <span :class="['px-2 text-12 text-secondary', !store.state?.hooksOff && 'mt-auto']"
          >Warden {{ store.state?.version }}</span
        >
      </nav>

      <div class="flex max-h-full min-w-0 grow-999 basis-130 flex-col">
        <div class="flex items-start justify-between gap-4 border-b border-divider px-7 pt-6 pb-4.5">
          <div class="flex flex-col gap-0.5">
            <h2 class="text-18 font-semibold">{{ title }}</h2>
            <span class="text-14 text-tertiary">{{ subtitle }}</span>
          </div>
          <div class="flex shrink-0 items-center gap-2">
            <WButton
              v-if="!searching && section === 'accounts'"
              variant="primary"
              :icon="Plus"
              @click="addOpen = true"
            >
              Add account
            </WButton>
            <WIconButton label="Close settings" :icon="X" @click="open = false" />
          </div>
        </div>

        <div class="flex flex-auto flex-col gap-7 overflow-y-auto px-7 pt-2 pb-7">
          <div v-if="searching" class="flex flex-col pt-4">
            <WList v-if="results.length">
              <WListItem v-for="r in results" :key="r.label" action @select="goTo(r)">
                <span class="flex min-w-0 flex-auto flex-col gap-0.5">
                  <span class="text-14 font-medium text-text">{{ r.label }}</span>
                  <span class="text-13 text-tertiary">{{ r.help }}</span>
                </span>
                <span
                  class="rounded-md bg-chip-off px-2 py-0.5 text-12 font-medium whitespace-nowrap text-chip-off-text"
                >
                  {{ sections.find((s) => s.id === r.s)?.label }}
                </span>
                <ChevronRight class="size-4 shrink-0 text-icon" aria-hidden="true" />
              </WListItem>
            </WList>
            <span v-else class="text-14 text-tertiary">Nothing matches that search.</span>
          </div>
          <GeneralSection v-else-if="section === 'general'" />
          <AccountsSection v-else-if="section === 'accounts'" />
        </div>
      </div>
    </div>
  </AppModal>
  <AddAccountModal v-model:open="addOpen" />
</template>
