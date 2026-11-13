<script setup lang="ts">
// Main on top, then Warden's accounts, each with one switch: it takes delegated work, and installs or removes the
// account's status hook. The Hooks column always says how each account's hooks are; the sign-in, usage and CLI say
// something only when something is wrong, in amber, with the fix beside it. Under the table, main's reserve and the
// work hours it shrinks with.
import { Check, Pencil, Trash2 } from 'lucide-vue-next';
import { computed, ref, watch } from 'vue';
import { api, type Account, type WorkHours } from '../api';
import AccountIcon from '../components/AccountIcon.vue';
import MoreMenu, { type MenuItem } from '../components/MoreMenu.vue';
import ToggleChip from '../components/ToggleChip.vue';
import WButton from '../components/WButton.vue';
import WSlider from '../components/WSlider.vue';
import WSwitch from '../components/WSwitch.vue';
import WTag from '../components/WTag.vue';
import WTable from '../components/WTable.vue';
import WTableRow from '../components/WTableRow.vue';
import WTooltip from '../components/WTooltip.vue';
import {
  FIX_LABEL,
  WEEKDAYS,
  accountProblems,
  clock,
  handOffPreview,
  hooksCell,
  type Fix,
  whereText,
  workHoursText,
} from '../format';
import { act, store } from '../store';
import AccountEditModal from './AccountEditModal.vue';

const accounts = computed(() =>
  store.state ? [...(store.state.main ? [store.state.main] : []), ...store.state.accounts] : [],
);
const error = ref<string | null>(null);

async function run(f: () => ReturnType<typeof api.state>): Promise<void> {
  error.value = await act(f);
}

function setUse(a: Account, on: boolean): void {
  void run(() => api.update(a.id, { enabled: on }));
}

function problems(a: Account, identity: boolean) {
  const all = accountProblems(a, { all: accounts.value, problems: store.state?.problems ?? [] });
  return all.filter((p) => !!p.identity === identity);
}

function hooks(a: Account) {
  return hooksCell(a, store.state?.hooksOff ?? false);
}

function fix(a: Account, f: Fix): void {
  switch (f) {
    case 'signInMain':
    case 'signInMainAgain':
      void run(api.signInMain);
      return;
    case 'signIn':
    case 'undo':
      void run(() => api.signIn(a.id));
      return;
    case 'accept':
      void run(() => api.acceptIdentity(a.id));
      return;
    case 'scan':
      void run(api.scanTools);
  }
}

// The reserve follows the drag and is saved once, where the thumb is let go. While it differs from the saved one, the
// hand-off point is worked out here with the server's formula; then the server's own is shown.
const reserve = ref(store.state?.settings.reserve ?? 25);
watch(
  () => store.state?.settings.reserve,
  (r) => {
    if (r !== undefined) reserve.value = r;
  },
);
async function saveReserve(value: number | number[]): Promise<void> {
  if (typeof value !== 'number') return;
  await run(() => api.settings({ reserve: value }));
  // Refused: the slider goes back to the reserve that still decides delegation.
  if (error.value) reserve.value = store.state?.settings.reserve ?? reserve.value;
}
const handOffAt = computed(() => {
  const s = store.state;
  if (!s) return 100;
  if (reserve.value === s.settings.reserve) return s.delegation.handOffAt;
  return handOffPreview(reserve.value, s.settings.workHours, s.main?.usage?.weekly?.resetsAt ?? null, store.now);
});

// Work hours: a switch, the days, and one range for the hours; each change is saved at once (the range once per drag).
const workHours = computed(() => store.state?.settings.workHours ?? null);
const FIRST: WorkHours = { days: [1, 2, 3, 4, 5], from: 9 * 60, to: 18 * 60 };
function saveWorkHours(w: WorkHours | null): void {
  void run(() => api.settings({ workHours: w }));
}
// The days as last clicked, until the server has answered every click: two quick clicks build on each other, and a
// refused one goes back to the saved days.
const draftDays = ref<number[] | null>(null);
const days = computed(() => draftDays.value ?? workHours.value?.days ?? []);
let dayChanges = 0;
async function toggleDay(day: number): Promise<void> {
  const w = workHours.value;
  if (!w) return;
  const now = days.value;
  if (now.includes(day) && now.length === 1) return;
  const next = now.includes(day) ? now.filter((d) => d !== day) : [...now, day].sort((a, b) => a - b);
  draftDays.value = next;
  const mine = ++dayChanges;
  await run(() => api.settings({ workHours: { ...w, days: next } }));
  if (mine === dayChanges) draftDays.value = null;
}
// The hours follow the drag and are saved where the thumb is let go. A new state from the server (a session event,
// say) only moves them when the saved hours themselves change, and never during a drag.
const hours = ref<number[]>([FIRST.from, FIRST.to]);
const dragging = ref(false);
watch(
  () => (workHours.value ? `${workHours.value.from}-${workHours.value.to}` : null),
  () => {
    const w = workHours.value;
    if (w && !dragging.value) hours.value = [w.from, w.to];
  },
  { immediate: true },
);
function moveHours(value: number | number[]): void {
  if (!Array.isArray(value)) return;
  dragging.value = true;
  hours.value = value;
}
async function saveHours(value: number | number[]): Promise<void> {
  dragging.value = false;
  const w = workHours.value;
  if (!w || !Array.isArray(value) || value[0] === undefined || value[1] === undefined) return;
  await run(() => api.settings({ workHours: { ...w, days: days.value, from: value[0]!, to: value[1]! } }));
  if (error.value && workHours.value) hours.value = [workHours.value.from, workHours.value.to];
}

// Rename and remove open a second modal on top.
const editing = ref<{ account: Account; mode: 'rename' | 'remove' } | null>(null);
const editOpen = computed({
  get: () => editing.value !== null,
  set: (v: boolean) => {
    if (!v) editing.value = null;
  },
});

function menu(a: Account): MenuItem[] {
  const rename: MenuItem = {
    label: 'Rename',
    icon: Pencil,
    run: () => (editing.value = { account: a, mode: 'rename' }),
  };
  if (a.id === 'main') return [rename];
  return [
    rename,
    { label: 'Remove', icon: Trash2, danger: true, run: () => (editing.value = { account: a, mode: 'remove' }) },
  ];
}

function identityLine(a: Account): string | null {
  const id = a.identity;
  if (!id) return null;
  return id.organization ? `${id.email}, ${id.organization}` : id.email;
}
</script>

<template>
  <div class="flex flex-col gap-3 pt-4">
    <p v-if="error" role="alert" class="text-13 font-medium text-attn">{{ error }}</p>

    <div v-if="accounts.length === 0" class="rounded-lg border border-divider-strong p-4 text-14 text-tertiary">
      No accounts yet.
    </div>

    <WTable v-else aria-label="Accounts" cols="grid-cols-account-list gap-3">
      <template #head>
        <span role="columnheader">Account</span>
        <span role="columnheader">Hooks</span>
        <span role="columnheader" class="text-center"><WTooltip text="Takes delegated work">Use</WTooltip></span>
        <span role="columnheader"><span class="sr-only">More</span></span>
      </template>
      <WTableRow v-for="a in accounts" :key="a.id" :off="!a.enabled" class="py-3">
        <div role="cell" class="flex min-w-0 items-center gap-3.5">
          <!-- Switched off: the icon is dimmed; the text stays at full contrast in grey, which opacity would break. -->
          <AccountIcon :account="a" size="lg" on="modal" :class="['self-start', !a.enabled && 'opacity-55']" />
          <div class="flex min-w-0 flex-auto flex-col gap-px">
            <span class="flex items-center gap-2">
              <span :class="['truncate text-14 font-semibold', !a.enabled && 'text-secondary']">{{ a.label }}</span>
              <WTag v-if="!a.enabled">Not used</WTag>
            </span>
            <!-- The folder is in the tooltip; a changed sign-in stands in the email's place. -->
            <WTooltip :text="a.folder" class="max-w-full min-w-0 self-start truncate text-left text-13 text-tertiary">
              {{ whereText(a) }}<template v-if="identityLine(a) && !a.identityChange">, {{ identityLine(a) }}</template>
            </WTooltip>
            <ul
              v-if="problems(a, true).length"
              class="mt-1.5 flex flex-col gap-1.5"
              :aria-label="`Sign-in of ${a.label}`"
            >
              <li v-for="(p, k) in problems(a, true)" :key="k" class="flex flex-wrap items-center gap-x-3 gap-y-1.5">
                <WTooltip
                  v-if="p.tooltip"
                  :text="p.tooltip"
                  class="min-w-0 text-left text-13 font-medium wrap-anywhere text-attn"
                  >{{ p.text }}</WTooltip
                >
                <span v-else class="min-w-0 text-13 font-medium text-attn">{{ p.text }}</span>
                <span v-if="p.fixes.length" class="flex gap-2">
                  <WButton
                    v-for="f in p.fixes"
                    :key="f"
                    :aria-label="`${FIX_LABEL[f]} for ${a.label}`"
                    @click="fix(a, f)"
                    >{{ FIX_LABEL[f] }}</WButton
                  >
                </span>
              </li>
            </ul>

            <ul
              v-if="problems(a, false).length"
              class="mt-2 flex flex-col gap-1.5"
              :aria-label="`Problems with ${a.label}`"
            >
              <li v-for="(p, k) in problems(a, false)" :key="k" class="flex flex-wrap items-center gap-x-3 gap-y-1.5">
                <span class="min-w-0 text-13 font-medium text-attn">{{ p.text }}</span>
                <span v-if="p.fixes.length" class="flex gap-2">
                  <WButton
                    v-for="f in p.fixes"
                    :key="f"
                    :aria-label="`${FIX_LABEL[f]} for ${a.label}`"
                    @click="fix(a, f)"
                    >{{ FIX_LABEL[f] }}</WButton
                  >
                </span>
              </li>
            </ul>
          </div>
        </div>

        <!-- Main's cell covers its routing and status hooks together; its tooltip says so. -->
        <span role="cell" class="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1.5" data-hooks>
          <component
            :is="a.id === 'main' ? WTooltip : 'span'"
            v-bind="a.id === 'main' ? { text: 'The routing hook and the status hook' } : {}"
            :class="[
              'inline-flex items-center gap-1.5 text-left text-13',
              hooks(a).tone === 'ok'
                ? 'text-secondary'
                : hooks(a).tone === 'attn'
                  ? 'font-medium text-attn'
                  : 'text-tertiary',
            ]"
          >
            <Check v-if="hooks(a).tone === 'ok'" class="size-4 shrink-0" aria-hidden="true" />{{ hooks(a).text }}
          </component>
          <WButton
            v-if="hooks(a).repair"
            :aria-label="`Repair the hooks of ${a.label}`"
            @click="run(() => api.repairHooks(a.id))"
            >Repair</WButton
          >
        </span>

        <span role="cell" class="flex justify-center">
          <WSwitch
            v-if="a.id !== 'main'"
            :model-value="a.enabled"
            :label="a.enabled ? `Stop using ${a.label}` : `Use ${a.label}`"
            @update:model-value="setUse(a, $event)"
          />
        </span>

        <span role="cell" class="flex">
          <MoreMenu :label="`More options for ${a.label}`" :items="menu(a)" />
        </span>
      </WTableRow>
    </WTable>

    <section v-if="store.state?.main" aria-label="Main's reserve" class="mt-4 flex flex-col">
      <div class="flex flex-wrap items-center justify-between gap-x-6 gap-y-3 border-b border-divider py-4">
        <div class="flex min-w-0 flex-auto basis-65 flex-col gap-0.5">
          <span id="reserve-label" class="cursor-default text-14 font-medium">Keep for my own work</span>
          <span class="text-13 text-tertiary"
            >Main's share of its week for your own work. Beyond it, work goes to the other accounts.</span
          >
        </div>
        <div class="flex min-w-0 flex-auto basis-60 flex-col gap-1">
          <span class="flex items-center gap-3">
            <WSlider
              v-model="reserve"
              labelledby="reserve-label"
              :min="0"
              :max="50"
              :step="5"
              :value-text="`${reserve}% of its week`"
              @commit="saveReserve"
            />
            <span class="text-13 whitespace-nowrap text-secondary tabular-nums">{{ reserve }}% of its week</span>
          </span>
          <span class="text-13 text-secondary tabular-nums" data-hand-off>Hands off at {{ handOffAt }}% now</span>
        </div>
      </div>

      <div class="flex flex-col gap-3 py-4">
        <div class="flex items-center justify-between gap-6">
          <div class="flex min-w-0 flex-auto flex-col gap-0.5">
            <label for="work-hours" class="cursor-pointer text-14 font-medium">My work hours</label>
            <span class="text-13 text-tertiary"
              >Main's reserve shrinks only during these hours, so evenings and weekends keep it.</span
            >
          </div>
          <WSwitch
            id="work-hours"
            :model-value="workHours !== null"
            label="My work hours"
            @update:model-value="saveWorkHours($event ? FIRST : null)"
          />
        </div>
        <template v-if="workHours">
          <div role="group" aria-label="Work days" class="flex flex-wrap gap-2">
            <template v-for="(name, k) in WEEKDAYS" :key="name">
              <WTooltip v-if="days.length === 1 && days.includes(k + 1)" text="At least one day" as-child>
                <ToggleChip :pressed="true" aria-disabled="true">{{ name }}</ToggleChip>
              </WTooltip>
              <ToggleChip v-else :pressed="days.includes(k + 1)" @click="toggleDay(k + 1)">{{ name }}</ToggleChip>
            </template>
          </div>
          <span class="flex items-center gap-3">
            <WSlider
              :model-value="hours"
              range
              label="Work hours"
              :thumb-labels="['Work hours start', 'Work hours end']"
              :value-texts="[clock(hours[0] ?? 0), clock(hours[1] ?? 0)]"
              :min="0"
              :max="1440"
              :step="30"
              class="max-w-120"
              @update:model-value="moveHours"
              @commit="saveHours"
            />
            <span class="text-13 whitespace-nowrap text-secondary tabular-nums" data-work-hours>{{
              workHoursText({ from: hours[0] ?? 0, to: hours[1] ?? 0 })
            }}</span>
          </span>
        </template>
      </div>
    </section>
  </div>
  <AccountEditModal v-model:open="editOpen" :account="editing?.account ?? null" :mode="editing?.mode ?? 'rename'" />
</template>
