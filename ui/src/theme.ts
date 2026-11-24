// The sidebar's theme button follows the system preference and saves changes through the shared settings API.
import { useMediaQuery } from '@vueuse/core';
import { computed } from 'vue';
import { api } from './api';
import { act, store } from './store';

export function useThemeSwitch() {
  // The same preference that CSS uses when the document's theme is left to the system.
  const systemIsDark = useMediaQuery('(prefers-color-scheme: dark)');
  const systemTheme = computed(() => (systemIsDark.value ? 'dark' : 'light'));
  const activeTheme = computed(() => {
    const theme = store.state?.settings.theme ?? 'system';
    return theme === 'system' ? systemTheme.value : theme;
  });

  function switchTheme(): Promise<string | null> {
    const opposite = systemIsDark.value ? 'light' : 'dark';
    const theme = activeTheme.value === systemTheme.value ? opposite : 'system';
    return act(() => api.settings({ theme }));
  }

  return { activeTheme, switchTheme };
}
