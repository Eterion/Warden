// Memory's scale and filter, shown immediately and saved in order through the settings API.
import { computed, ref } from 'vue';
import { ApiError, api, type Settings } from './api';
import { DEFAULT_MEMORY_FILTER } from './memoryGroups';
import { apply, store } from './store';

type MemorySettings = Pick<Settings, 'memoryScale' | 'memoryFilter'>;

const pending = ref<Partial<MemorySettings>>({});
const error = ref('');
let saving: Promise<void> = Promise.resolve();
let queued = 0;

export const memorySettings = computed<MemorySettings>(() => ({
  memoryScale: store.state?.settings.memoryScale ?? 'sessions',
  memoryFilter: store.state?.settings.memoryFilter ?? DEFAULT_MEMORY_FILTER,
  ...pending.value,
}));

export const memorySettingsError = computed(() => error.value);

/** Keep choices responsive and save them in order, including when the user leaves and reopens Memory. */
export function saveMemorySettings(patch: Partial<MemorySettings>): void {
  pending.value = { ...pending.value, ...patch };
  error.value = '';
  queued++;
  saving = saving.then(async () => {
    try {
      const state = await api.settings(patch);
      if (!store.state || state.revision >= store.state.revision) apply(state);
    } catch (e) {
      if (!(e instanceof ApiError)) store.connected = false;
      error.value = e instanceof ApiError ? e.message : 'Warden could not be reached.';
    } finally {
      if (--queued === 0) pending.value = {};
    }
  });
}
