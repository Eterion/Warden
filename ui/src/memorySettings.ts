// Save Memory settings in order. Build each filter change from the settings that the server accepted.
import { computed, ref, shallowRef } from 'vue';
import { ApiError, api, type MemoryFilter, type MemoryScale, type Settings } from './api';
import { DEFAULT_MEMORY_FILTER } from './memoryGroups';
import { apply, store } from './store';

type MemorySettings = Pick<Settings, 'memoryScale' | 'memoryFilter'>;

const pendingScale = shallowRef<{ scale: MemoryScale } | null>(null);
const error = ref('');
let saving = Promise.resolve(true);

export const memorySettings = computed<MemorySettings>(() => ({
  memoryScale: pendingScale.value?.scale ?? store.state?.settings.memoryScale ?? 'sessions',
  memoryFilter: store.state?.settings.memoryFilter ?? DEFAULT_MEMORY_FILTER,
}));

export const memorySettingsError = computed(() => error.value);

function saveMemorySettings(patch: () => Partial<MemorySettings>): Promise<boolean> {
  error.value = '';
  saving = saving.then(async () => {
    try {
      const state = await api.settings(patch());
      if (!store.state || state.revision >= store.state.revision) apply(state);
      return true;
    } catch (e) {
      if (!(e instanceof ApiError)) store.connected = false;
      error.value = e instanceof ApiError ? e.message : 'Warden could not be reached.';
      return false;
    }
  });
  return saving;
}

export async function saveMemoryScale(scale: MemoryScale): Promise<void> {
  const choice = { scale };
  pendingScale.value = choice;
  await saveMemorySettings(() => ({ memoryScale: scale }));
  if (pendingScale.value === choice) pendingScale.value = null;
}

export function saveMemoryFilter(change: (current: MemoryFilter) => MemoryFilter): Promise<boolean> {
  return saveMemorySettings(() => ({ memoryFilter: change(memorySettings.value.memoryFilter) }));
}
