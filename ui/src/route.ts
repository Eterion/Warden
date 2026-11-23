// Which page shows and which modal is open, kept in the address (# for the Overview, #/memory for today's memory,
// #/memory/2026-11-20 for a past day's; #/settings/accounts, #/settings/accounts/add, #/settings/general, or
// #/add-account for Add account on its own, from the sidebar), so each state has its own link and the back button goes
// back to it. A modal opens over the page it was opened on, which stays underneath.

import { useEventListener } from '@vueuse/core';
import { computed, ref, watch } from 'vue';

export type Section = 'general' | 'accounts';
export type Page = 'overview' | 'memory';

const SECTIONS: Section[] = ['general', 'accounts'];

const hash = ref(typeof location === 'undefined' ? '' : location.hash);
if (typeof window !== 'undefined') useEventListener(window, 'hashchange', () => (hash.value = location.hash));

const parts = computed(() => hash.value.replace(/^#\/?/, '').split('/'));

interface PageAt {
  page: Page;
  /** The Memory page's day (yyyy-MM-dd); null: today. */
  day: string | null;
}

/** The addresses that open a modal over the page. */
const MODALS = ['settings', 'add-account'];
/** The page under a modal: the one shown when it opened. */
const lastPage = ref<PageAt>({ page: 'overview', day: null });
const pageNow = computed<PageAt>(() => {
  const [first, second] = parts.value;
  if (MODALS.includes(first ?? '')) return lastPage.value;
  if (first === 'memory') return { page: 'memory', day: second && /^\d{4}-\d{2}-\d{2}$/.test(second) ? second : null };
  return { page: 'overview', day: null };
});
// At once, so a modal opened right after a page change keeps that page under it.
watch(pageNow, (p) => (lastPage.value = p), { immediate: true, flush: 'sync' });

export const route = {
  settingsOpen: computed(() => parts.value[0] === 'settings'),
  section: computed<Section>(() => {
    const s = parts.value[1] as Section | undefined;
    return s && SECTIONS.includes(s) ? s : 'general';
  }),
  /** Add account, over Settings or on its own. */
  addOpen: computed(
    () =>
      parts.value[0] === 'add-account' ||
      (parts.value[0] === 'settings' && parts.value[1] === 'accounts' && parts.value[2] === 'add'),
  ),
  /** Add account on its own, without Settings underneath. */
  addAlone: computed(() => parts.value[0] === 'add-account'),
  page: computed(() => pageNow.value.page),
  /** The day the Memory page shows (yyyy-MM-dd); null: today. */
  day: computed(() => pageNow.value.day),
};

export function go(path: string): void {
  location.hash = path ? `#/${path}` : '';
  hash.value = location.hash;
}

/** Opens Add account on its own (the sidebar's Add account). */
export function openAddAccount(): void {
  go('add-account');
}

/** Where Settings gives focus back as it closes, when what opened it is gone by then (a menu item). */
let settingsReturn: (() => HTMLElement | null) | null = null;

/** Opens Settings in a section; `returnTo` names where focus goes as it closes, in place of what had focus. */
export function openSettings(section: Section = 'general', returnTo?: () => HTMLElement | null): void {
  settingsReturn = returnTo ?? null;
  go(`settings/${section}`);
}

/** The element Settings was told to give focus back to, once (Settings asks as it closes). */
export function takeSettingsReturn(): HTMLElement | null {
  const to = settingsReturn?.() ?? null;
  settingsReturn = null;
  return to;
}

/** The address of the page under a modal, so closing it goes back there. */
function pageHash(): string {
  const p = lastPage.value;
  if (p.page === 'memory') return p.day ? `#/memory/${p.day}` : '#/memory';
  return '';
}

export function closeModal(): void {
  // Back to the page underneath; the Overview's address stays clean rather than an empty "#".
  const to = pageHash();
  history.replaceState(null, '', location.pathname + location.search + to);
  hash.value = to;
}
