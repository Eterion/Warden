<script setup lang="ts">
// Settings, Appearance: the theme and whether Overview and Memory animate when switching pages.
import { Monitor, Moon, Sun } from '@lucide/vue';
import { computed, ref } from 'vue';
import { api, type Theme } from '../api';
import SegmentedControl from '../components/SegmentedControl.vue';
import WSwitch from '../components/WSwitch.vue';
import { act, store } from '../store';

const error = ref<string | null>(null);

const theme = computed<Theme>({
  get: () => store.state?.settings.theme ?? 'system',
  set: (v) => void run(() => api.settings({ theme: v })),
});
const pageTransitions = computed<boolean>({
  get: () => store.state?.settings.pageTransitions ?? true,
  set: (v) => void run(() => api.settings({ pageTransitions: v })),
});

async function run(f: () => ReturnType<typeof api.settings>): Promise<void> {
  error.value = await act(f);
}
</script>

<template>
  <div class="flex flex-col gap-8 pt-4">
    <p v-if="error" role="alert" class="text-13 font-medium text-attn">{{ error }}</p>

    <section class="flex flex-col">
      <div class="flex flex-wrap items-center justify-between gap-x-6 gap-y-3 border-b border-divider py-4">
        <div class="flex flex-auto basis-65 flex-col gap-0.5">
          <span class="text-14 font-medium">Theme</span>
          <span class="text-13 text-tertiary">System follows the Windows setting.</span>
        </div>
        <SegmentedControl
          v-model="theme"
          label="Theme"
          :options="[
            { value: 'system', label: 'System', icon: Monitor },
            { value: 'light', label: 'Light', icon: Sun },
            { value: 'dark', label: 'Dark', icon: Moon },
          ]"
        />
      </div>
      <div class="flex items-center justify-between gap-6 py-4">
        <div class="flex min-w-0 flex-auto flex-col gap-0.5">
          <label for="page-transitions" class="cursor-pointer text-14 font-medium">Page transitions</label>
          <span class="text-13 text-tertiary"
            >Fade when switching between Overview and Memory. Turn off to switch instantly.</span
          >
        </div>
        <WSwitch id="page-transitions" v-model="pageTransitions" label="Page transitions" />
      </div>
    </section>
  </div>
</template>
