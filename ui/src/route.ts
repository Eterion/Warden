// Which modal is open, kept in the address (#/settings/accounts, #/settings/accounts/add,
// or #/settings/general), so each state has its own link and the back button closes it.

import { useEventListener } from '@vueuse/core';
import { computed, ref } from 'vue';

export type Section = 'general' | 'accounts';

const SECTIONS: Section[] = ['general', 'accounts'];

const hash = ref(typeof location === 'undefined' ? '' : location.hash);
if (typeof window !== 'undefined') useEventListener(window, 'hashchange', () => (hash.value = location.hash));

const parts = computed(() => hash.value.replace(/^#\/?/, '').split('/'));

export const route = {
  settingsOpen: computed(() => parts.value[0] === 'settings'),
  section: computed<Section>(() => {
    const s = parts.value[1] as Section | undefined;
    return s && SECTIONS.includes(s) ? s : 'general';
  }),
  addOpen: computed(() => parts.value[0] === 'settings' && parts.value[1] === 'accounts' && parts.value[2] === 'add'),
};

export function go(path: string): void {
  location.hash = path ? `#/${path}` : '';
  hash.value = location.hash;
}

export function openSettings(section: Section = 'general'): void {
  go(`settings/${section}`);
}

export function closeModal(): void {
  // Leave the address clean rather than an empty "#".
  history.replaceState(null, '', location.pathname + location.search);
  hash.value = '';
}
