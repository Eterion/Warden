<script setup lang="ts">
// Settings, App: the usage interval, reset warning, data and logs. The desktop app adds
// Keep running in the tray, Start with Windows and Open log folder. The browser demo shows the log folder's path.
import { FolderOpen } from '@lucide/vue';
import { computed, ref, watch } from 'vue';
import { api } from '../api';
import SegmentedControl from '../components/SegmentedControl.vue';
import WButton from '../components/WButton.vue';
import WSlider from '../components/WSlider.vue';
import WSwitch from '../components/WSwitch.vue';
import { go } from '../route';
import { act, store } from '../store';

const error = ref<string | null>(null);

const interval = computed<string>({
  get: () => String(store.state?.settings.usageIntervalMin ?? 5),
  set: (v) => void run(() => api.settings({ usageIntervalMin: Number(v) })),
});
const keepInTray = computed<boolean>({
  get: () => store.state?.settings.keepInTray ?? false,
  set: (v) => void run(() => api.settings({ keepInTray: v })),
});
const startWithWindows = computed<boolean>({
  get: () => store.state?.desktop?.startWithWindows ?? false,
  set: (v) => void run(() => api.settings({ startWithWindows: v })),
});

// The warning follows the drag and is saved once, where the thumb is let go; a refused one goes back to the saved one.
const warnDays = ref(store.state?.settings.resetWarningDays ?? 3);
watch(
  () => store.state?.settings.resetWarningDays,
  (d) => {
    if (d !== undefined) warnDays.value = d;
  },
);
const warnDaysText = computed(() => (warnDays.value === 1 ? '1 day' : `${warnDays.value} days`));
async function saveWarnDays(value: number | number[]): Promise<void> {
  if (typeof value !== 'number') return;
  await run(() => api.settings({ resetWarningDays: value }));
  if (error.value) warnDays.value = store.state?.settings.resetWarningDays ?? warnDays.value;
}

async function run(f: () => ReturnType<typeof api.settings>): Promise<void> {
  error.value = await act(f);
}

async function runSetupAgain(): Promise<void> {
  error.value = await act(api.restartSetup);
  if (!error.value) go('');
}
</script>

<template>
  <div class="flex flex-col gap-8 pt-4">
    <p v-if="error" role="alert" class="text-13 font-medium text-attn">{{ error }}</p>

    <section class="flex flex-col">
      <div class="flex flex-wrap items-center justify-between gap-x-6 gap-y-3 border-b border-divider py-4">
        <div class="flex flex-auto basis-65 flex-col gap-0.5">
          <span id="interval-label" class="text-14 font-medium">Read usage every</span>
          <span class="text-13 text-tertiary"
            >One small request per account; Codex usage is read from its logs. No model tokens.</span
          >
        </div>
        <SegmentedControl
          v-model="interval"
          label="Read usage every"
          :options="[
            { value: '2', label: '2 min' },
            { value: '5', label: '5 min' },
            { value: '10', label: '10 min' },
            { value: '30', label: '30 min' },
          ]"
        />
      </div>
      <div class="flex flex-wrap items-center justify-between gap-x-6 gap-y-3 border-b border-divider py-4">
        <div class="flex min-w-0 flex-auto basis-65 flex-col gap-0.5">
          <span id="reset-warning-label" class="cursor-default text-14 font-medium">Warn before a reset expires</span>
          <span class="text-13 text-tertiary"
            >A limit reset this close to expiring shows in amber on the Overview.</span
          >
        </div>
        <span class="flex min-w-0 flex-auto basis-60 items-center gap-3">
          <WSlider
            v-model="warnDays"
            labelledby="reset-warning-label"
            :min="1"
            :max="14"
            :step="1"
            :value-text="warnDaysText"
            @commit="saveWarnDays"
          />
          <span class="text-13 whitespace-nowrap text-secondary tabular-nums" data-reset-warning>{{
            warnDaysText
          }}</span>
        </span>
      </div>
      <template v-if="store.state?.desktop">
        <div class="flex items-center justify-between gap-6 border-b border-divider py-4">
          <div class="flex min-w-0 flex-auto flex-col gap-0.5">
            <label for="keep-in-tray" class="cursor-pointer text-14 font-medium">Keep running in the tray</label>
            <span class="text-13 text-tertiary">
              {{
                keepInTray
                  ? 'Closing the window leaves Warden in the tray, still watching your sessions.'
                  : 'Closing the window quits Warden. Routing still works; live monitoring stops.'
              }}
            </span>
          </div>
          <WSwitch id="keep-in-tray" v-model="keepInTray" label="Keep running in the tray" />
        </div>
        <div class="flex items-center justify-between gap-6 border-b border-divider py-4">
          <div class="flex min-w-0 flex-auto flex-col gap-0.5">
            <label for="start-with-windows" class="cursor-pointer text-14 font-medium">Start with Windows</label>
            <span class="text-13 text-tertiary">
              {{ keepInTray ? 'Start Warden in the tray when you sign in.' : 'Open Warden when you sign in.' }}
            </span>
          </div>
          <WSwitch id="start-with-windows" v-model="startWithWindows" label="Start with Windows" />
        </div>
      </template>
      <div class="flex flex-wrap items-center gap-2.5 border-b border-divider py-4">
        <div class="flex min-w-0 flex-auto basis-65 flex-col gap-0.5">
          <span class="text-14 font-medium">Warden's data</span>
          <span class="font-mono text-12 wrap-anywhere text-tertiary">{{ store.state?.dataDir }}</span>
        </div>
        <WButton @click="runSetupAgain">Run setup again</WButton>
      </div>
      <div class="flex flex-wrap items-center gap-2.5 py-4">
        <div class="flex min-w-0 flex-auto basis-65 flex-col gap-0.5">
          <span class="text-14 font-medium">Log files</span>
          <span class="font-mono text-12 wrap-anywhere text-tertiary">{{ store.state?.logDir }}</span>
        </div>
        <WButton v-if="store.state?.desktop" :icon="FolderOpen" @click="run(api.openLogFolder)"
          >Open log folder</WButton
        >
      </div>
    </section>
  </div>
</template>
