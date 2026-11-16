<script setup lang="ts">
// The Overview: the title, the accounts in use with their limits (main's hand-off point marked on its weekly bar), the last 2 hours of sessions and the
// "now" chip, then every account's sessions in one table, each delegated session under the main session that started
// it. Checking accounts shows only their sessions, under their faded ancestors; the chip in the Sessions header hides
// ended and not-known sessions (remembered), an ancestor of a shown one staying faded. Beside them (under them in a
// narrow window), the limit resets the accounts can use, while there are any; one expiring within the warning days set
// in Settings › General in amber.
import { Check, Hourglass } from 'lucide-vue-next';
import { computed, ref } from 'vue';
import { api, type Account, type Session } from '../api';
import AccountIcon from '../components/AccountIcon.vue';
import ActivitySparkline from '../components/ActivitySparkline.vue';
import NowChip from '../components/NowChip.vue';
import ProviderMark from '../components/ProviderMark.vue';
import RemovableChip from '../components/RemovableChip.vue';
import StateIcon from '../components/StateIcon.vue';
import SubagentsBadge from '../components/SubagentsBadge.vue';
import UsageBar from '../components/UsageBar.vue';
import WTooltip from '../components/WTooltip.vue';
import WButton from '../components/WButton.vue';
import WTable from '../components/WTable.vue';
import WTableRow from '../components/WTableRow.vue';
import { vLabelState } from '../components/labels';
import WList from '../components/WList.vue';
import WListItem from '../components/WListItem.vue';
import {
  IDLE_LOGIN,
  expectedSignIn,
  expiryText,
  extraText,
  limitView,
  loginIdle,
  readingNote,
  resetRows,
  signedInAs,
} from '../format';
import { go } from '../route';
import {
  STATE_LABEL,
  isQuiet,
  runningFor,
  sessionName,
  sessionTree,
  shownState,
  sparkPoints,
  subagentsLabel,
  tally,
  type Point,
} from '../sessions';
import { act, store, usedAccounts } from '../store';

const accounts = computed(() => usedAccounts(store.state));

// How many days before a reset expires its row warns: Settings › General.
const warnDays = computed(() => store.state?.settings.resetWarningDays ?? 3);
const resets = computed(() => resetRows(accounts.value, store.now, warnDays.value));

/** The Extra usage column shows only while some account has spent some. */
const showExtra = computed(() => accounts.value.some((a) => extraText(a.provider, a.usage) !== null));

/** Main's hand-off point on its weekly bar, while it is below 100%. */
function mark(a: Account, k: number): number | undefined {
  const at = store.state?.delegation.handOffAt;
  return k === 1 && a.id === 'main' && at !== undefined && at < 100 ? at : undefined;
}

/** Accounts checked to filter the sessions; none checked shows all. */
const checked = ref<string[]>([]);
const filter = computed(() => checked.value.filter((id) => accounts.value.some((a) => a.id === id)));

function toggle(id: string): void {
  checked.value = checked.value.includes(id) ? checked.value.filter((x) => x !== id) : [...checked.value, id];
}

const byId = computed(
  () =>
    new Map([...(store.state?.main ? [store.state.main] : []), ...(store.state?.accounts ?? [])].map((a) => [a.id, a])),
);

/** Sessions of accounts in use, before the filter. */
const allSessions = computed(() => {
  const used = new Set(accounts.value.map((a) => a.id));
  return (store.state?.sessions ?? []).filter((s) => used.has(s.accountId));
});

const DONE_SHOWN_MS = 10 * 60_000;

/** Sessions still shown: an ended one for its last 10 minutes, as long as the core keeps it (and its children's links). */
const liveSessions = computed(() =>
  allSessions.value.filter((s) => s.endedAt === null || store.now - s.endedAt < DONE_SHOWN_MS),
);

/** The sessions after the account filter. */
const filtered = computed(() =>
  liveSessions.value.filter((s) => filter.value.length === 0 || filter.value.includes(s.accountId)),
);

/** Whether ended and not-known sessions are shown: remembered, set only by the checkbox in the Sessions header. */
const showQuiet = computed(() => store.state?.settings.showQuietSessions ?? true);
/** How many ended and not-known sessions there are now, after the filter: the checkbox shows while there is one. */
const quietCount = computed(() => filtered.value.filter(isQuiet).length);

function toggleQuiet(): void {
  void act(() => api.settings({ showQuietSessions: !showQuiet.value }));
}

/**
 * The sessions shown, after the filter and with ended and not-known ones left out while hidden, as a tree. While
 * accounts are checked or sessions are hidden, the shown sessions' ancestors on any account in use stand in their
 * places, faded.
 */
const rows = computed(() =>
  sessionTree(
    showQuiet.value ? filtered.value : filtered.value.filter((s) => !isQuiet(s)),
    filter.value.length || !showQuiet.value ? liveSessions.value : undefined,
  ),
);
const shownTally = computed(() => tally(rows.value.filter((r) => !r.faded).map((r) => r.session)));
/**
 * The rows as the table draws them: `dim` when the whole row shows faded, as a parent shown only for the sessions under
 * it, or a session that has ended or whose state is not known.
 */
const tableRows = computed(() =>
  rows.value.map((r) => {
    const shown = shownState(r.session);
    return { ...r, dim: r.faded || shown === 'done' || shown === 'unknown' };
  }),
);

const points = computed(() => {
  const map = new Map<string, Point[]>();
  for (const a of accounts.value) {
    const bins = store.state?.activity.find((x) => x.accountId === a.id)?.bins ?? [];
    map.set(a.id, sparkPoints(bins, store.now));
  }
  return map;
});
/** One scale for all accounts. */
const most = computed(() => Math.max(1, ...[...points.value.values()].flat().map((p) => p.working)));

function row(a: Account) {
  const u = a.usage;
  return {
    five: u?.fiveHour ? limitView(u.fiveHour, store.now) : null,
    week: u?.weekly ? limitView(u.weekly, store.now) : null,
    noFive: !!u && !u.fiveHour,
    extra: extraText(a.provider, u),
    note: readingNote(a, store.state?.settings.usageIntervalMin ?? 5, store.now),
    signedIn: signedInAs(a, store.state?.accounts ?? []),
    // Sessions signed in as someone else are shown, but count for no account.
    now: tally(allSessions.value.filter((s) => s.accountId === a.id && !s.signedInAs)),
  };
}

/** Quiet text: a step darker on a checked row, so it keeps 4.5:1 on the row's tint. */
function muted(a: Account): string {
  return filter.value.includes(a.id) ? 'text-secondary' : 'text-tertiary';
}

function sessionAccount(s: Session): Account | undefined {
  return byId.value.get(s.accountId);
}
</script>

<template>
  <div class="flex flex-col gap-4.5">
    <header class="flex items-center gap-3">
      <h1 class="text-20 font-semibold">Overview</h1>
    </header>

    <p v-if="!store.state && !store.connected" class="text-14 text-secondary">
      Warden's core is not running. Start it, and this page fills in by itself.
    </p>

    <div v-else-if="store.state" class="flex flex-wrap items-start gap-x-6 gap-y-4.5">
      <div class="flex min-w-0 grow-999 basis-192 flex-col gap-4.5">
        <section aria-labelledby="accounts-heading" class="flex flex-col gap-3">
          <div class="flex flex-col gap-0.5">
            <div class="flex flex-wrap items-baseline gap-x-4 gap-y-2">
              <h2 id="accounts-heading" class="text-16 font-semibold">Accounts</h2>
              <div
                v-if="accounts.length"
                aria-label="Activity line"
                class="ml-auto flex flex-wrap items-center gap-x-3.5 gap-y-1 text-12 text-secondary"
              >
                <span class="inline-flex items-center gap-1.5">
                  <span aria-hidden="true" class="h-0 w-3.5 border-t-2 border-act-working"></span>Sessions working
                </span>
                <span class="inline-flex items-center gap-1.5">
                  <span aria-hidden="true" class="size-1.75 rounded-full bg-attn-icon"></span>Waited for you
                </span>
              </div>
            </div>
          </div>

          <div
            v-if="accounts.length === 0"
            class="flex flex-col items-start gap-3 rounded-lg border border-divider-strong p-4"
          >
            <p class="text-14 text-secondary">No accounts yet.</p>
            <WButton variant="primary" @click="go('settings/accounts/add')">Add account</WButton>
          </div>

          <WTable
            v-else
            on="panel"
            aria-labelledby="accounts-heading"
            :cols="`${showExtra ? 'grid-cols-overview' : 'grid-cols-overview-no-extra'} gap-x-4 gap-y-3`"
            :class="showExtra ? 'min-w-overview' : 'min-w-overview-no-extra'"
          >
            <template #head>
              <span role="columnheader"><span class="sr-only">Show only its sessions</span></span>
              <span role="columnheader">Account</span>
              <span role="columnheader">5-hour limit</span>
              <span role="columnheader">Weekly limit</span>
              <span v-if="showExtra" role="columnheader">Extra usage</span>
              <span role="columnheader">Last 2 hours, then now</span>
            </template>
            <WTableRow
              v-for="a in accounts"
              :key="a.id"
              :class="[
                'group cursor-pointer py-2.5',
                filter.includes(a.id)
                  ? 'bg-selected-bg hov:bg-selected-hover press:bg-selected-press'
                  : 'hov:bg-row-hover press:bg-row-press',
              ]"
              @click="toggle(a.id)"
            >
              <span role="cell" class="inline-flex items-center justify-center">
                <button
                  type="button"
                  role="checkbox"
                  :aria-checked="filter.includes(a.id)"
                  :aria-label="`Show only the sessions of ${a.label}`"
                  :class="[
                    'inline-flex size-4 cursor-pointer items-center justify-center rounded-sm focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-indicator',
                    filter.includes(a.id)
                      ? 'bg-primary text-on-primary group-hov:bg-primary-hover group-press:bg-primary-press'
                      : 'border-2 border-control group-hov:bg-chip-off group-press:bg-press',
                  ]"
                >
                  <Check v-if="filter.includes(a.id)" class="size-3" :stroke-width="3" aria-hidden="true" />
                </button>
              </span>

              <div role="cell" class="flex min-w-0 items-center gap-3">
                <AccountIcon :account="a" />
                <span class="flex min-w-0 flex-col">
                  <span class="truncate font-semibold">{{ a.label }}</span>
                  <!-- A changed sign-in takes the email line's place; the expected email is in its tooltip. -->
                  <WTooltip
                    v-if="row(a).signedIn"
                    :text="expectedSignIn(a)"
                    class="max-w-full min-w-0 text-left text-12 font-medium wrap-anywhere text-attn"
                    @click.stop
                    >{{ row(a).signedIn }}</WTooltip
                  >
                  <span v-else-if="a.identity" :class="['truncate text-12', muted(a)]" :title="a.identity.email">{{
                    a.identity.email
                  }}</span>
                </span>
              </div>

              <div
                v-for="(limit, k) in [row(a).five, row(a).week]"
                :key="k"
                role="cell"
                class="flex min-w-0 flex-col gap-1.25"
              >
                <span v-if="k === 0 && row(a).note && !row(a).note!.idle" class="sr-only"
                  >{{ [row(a).note!.problem, row(a).note!.age].filter(Boolean).join('. ') }}.</span
                >
                <span v-if="mark(a, k) !== undefined" class="sr-only">Hands off at {{ mark(a, k) }}%.</span>
                <template v-if="limit">
                  <span class="flex flex-wrap justify-between gap-x-2 text-12">
                    <span class="font-semibold whitespace-nowrap tabular-nums">{{ limit.used }}</span>
                    <span :class="['whitespace-nowrap', muted(a)]">{{ limit.reset }}</span>
                  </span>
                  <UsageBar :percent="limit.percent" :level="loginIdle(a) ? 'idle' : limit.level" :mark="mark(a, k)" />
                </template>
                <span v-else :class="['text-12', muted(a)]">
                  {{ k === 0 && row(a).noFive ? 'No 5-hour limit' : 'Not read yet' }}
                </span>
                <!-- An idle login's age, in the 5-hour cell so it can take focus: its tooltip says why it is grey. -->
                <WTooltip
                  v-if="k === 0 && row(a).note?.idle"
                  :text="IDLE_LOGIN"
                  :class="['self-start text-left text-12', muted(a)]"
                  @click.stop
                  >{{ row(a).note!.age }}</WTooltip
                >
              </div>

              <span v-if="showExtra" role="cell" class="text-12 text-secondary tabular-nums">{{ row(a).extra }}</span>

              <div role="cell" class="flex min-w-0 items-center gap-4">
                <ActivitySparkline :points="points.get(a.id) ?? []" :most="most" :name="a.label" />
                <NowChip :tally="row(a).now" />
              </div>

              <!-- Under the limits, only when it matters: a problem reading usage, or an old reading. Shown across both
                 limit columns; assistive technology hears it in the 5-hour cell, so the table keeps its columns. -->
              <p
                v-if="row(a).note && !row(a).note!.idle"
                aria-hidden="true"
                class="col-span-2 col-start-3 -mt-1.5 flex min-w-0 flex-wrap gap-x-2 text-12"
              >
                <span v-if="row(a).note!.problem" class="font-medium text-attn">{{ row(a).note!.problem }}</span>
                <span v-if="row(a).note!.age" :class="muted(a)">{{ row(a).note!.age }}</span>
              </p>
            </WTableRow>
          </WTable>
        </section>

        <section v-if="accounts.length" aria-labelledby="sessions-heading" class="flex flex-col gap-3">
          <div class="flex flex-wrap items-baseline gap-x-4 gap-y-2">
            <h2 id="sessions-heading" class="text-16 font-semibold">Sessions</h2>
            <div class="flex flex-wrap items-baseline gap-1.5 text-13">
              <RemovableChip
                v-for="id in filter"
                :key="id"
                size="sm"
                :label="`Stop showing only the sessions of ${byId.get(id)?.label}`"
                @click="toggle(id)"
              >
                {{ byId.get(id)?.label }}
              </RemovableChip>
              <WButton v-if="filter.length" variant="link" @click="checked = []">Show all</WButton>
            </div>
            <div class="ml-auto flex flex-wrap items-center gap-x-5 gap-y-1 text-12 text-secondary">
              <div aria-label="Sessions shown by state" class="flex flex-wrap items-center gap-x-3.5 gap-y-1">
                <span class="inline-flex items-center gap-1.5">
                  <StateIcon state="working" size="sm" />
                  <strong class="font-semibold text-text tabular-nums">{{ shownTally.working }}</strong> working
                </span>
                <span class="inline-flex items-center gap-1.5">
                  <StateIcon state="waiting" size="sm" />
                  <strong class="font-semibold text-text tabular-nums">{{ shownTally.waiting }}</strong> waiting for you
                </span>
                <span class="inline-flex items-center gap-1.5">
                  <StateIcon state="idle" size="sm" />
                  <strong class="font-semibold text-text tabular-nums">{{ shownTally.idle }}</strong> idle
                </span>
              </div>
              <!-- Checked shows ended and not-known sessions; its label (with the count) names it, and hovering or
                 pressing the label lights the box as one control. -->
              <span v-if="quietCount" class="inline-flex items-center gap-1.5" data-quiet-toggle>
                <button
                  id="quiet-sessions"
                  v-label-state
                  type="button"
                  role="checkbox"
                  :aria-checked="showQuiet"
                  :class="[
                    'inline-flex size-4 cursor-pointer items-center justify-center rounded-sm focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-indicator',
                    showQuiet
                      ? 'bg-primary text-on-primary hov:bg-primary-hover press:bg-primary-press'
                      : 'border-2 border-control hov:bg-chip-off press:bg-press',
                  ]"
                  @click="toggleQuiet"
                >
                  <Check v-if="showQuiet" class="size-3" :stroke-width="3" aria-hidden="true" />
                </button>
                <label for="quiet-sessions" class="cursor-pointer"
                  >Ended and not known <span class="tabular-nums">{{ quietCount }}</span></label
                >
              </span>
            </div>
          </div>

          <WTable on="panel" aria-labelledby="sessions-heading" cols="grid-cols-sessions gap-3" class="min-w-sessions">
            <template #head>
              <span role="columnheader"><span class="sr-only">State</span></span>
              <span role="columnheader">Session</span>
              <span role="columnheader">Doing now</span>
              <span role="columnheader">Project</span>
              <span role="columnheader">Account</span>
              <span role="columnheader">Subagents</span>
              <span role="columnheader">ID</span>
              <span role="columnheader" class="text-right">Running for</span>
            </template>
            <!-- A dimmed row (a parent shown only for the sessions under it while accounts are checked or ended and
               not-known sessions are hidden, or a session that has ended or whose state is not known): its text in the
               tertiary colour, its icons dimmed, no amber. Only a parent shown for its children says so to screen readers; the others' state is heard. -->
            <WTableRow
              v-for="{ session: s, depth, parent, rails, last, faded, dim } in tableRows"
              :key="`${s.provider}:${s.id}`"
              :class="[
                'py-2.25 text-13',
                dim ? 'icons-faded text-tertiary' : s.state === 'waiting' && s.source === 'hook' && 'bg-attn-bg',
              ]"
            >
              <span
                role="cell"
                :aria-label="STATE_LABEL[shownState(s)]"
                :title="STATE_LABEL[shownState(s)]"
                class="inline-flex items-center justify-center"
              >
                <StateIcon :state="shownState(s)" />
              </span>
              <!-- A delegated session sits under the main session that started it: one step in per level, with a thin
                 connector through the row's top border and padding to the rows above and below. Each part is drawn
                 once (the colour is translucent): a line through the row where a later sibling still needs it, and
                 this row's own branch off it, a straight stub, or the rounded elbow that ends the line at the last
                 sibling. The lines are decoration; screen readers hear who started it. -->
              <span role="cell" class="flex min-w-0 items-center self-stretch">
                <span v-for="level in depth" :key="level" aria-hidden="true" class="relative w-6 shrink-0 self-stretch">
                  <span
                    v-if="level < depth ? rails[level - 1] : !last"
                    data-line="rail"
                    class="absolute -top-2.5 -bottom-2.25 left-2 border-l border-divider-strong"
                  ></span>
                  <span
                    v-if="level === depth && !last"
                    data-line="stub"
                    class="absolute bottom-1/2 left-2.25 w-2.75 border-b border-divider-strong"
                  ></span>
                  <span
                    v-else-if="level === depth"
                    data-line="elbow"
                    class="absolute -top-2.5 bottom-1/2 left-2 w-3 rounded-bl-sm border-b border-l border-divider-strong"
                  ></span>
                </span>
                <span class="min-w-0 truncate font-medium" :title="sessionName(s)"
                  >{{ sessionName(s) }}<span v-if="parent" class="sr-only">. Started by {{ sessionName(parent) }}</span
                  ><span v-if="faded" class="sr-only">. Shown only for the sessions under it</span></span
                >
              </span>
              <span
                role="cell"
                :class="[
                  'min-w-0 truncate',
                  dim
                    ? 'text-tertiary'
                    : s.state === 'waiting'
                      ? 'font-medium text-attn'
                      : s.state === 'working'
                        ? 'text-secondary'
                        : 'text-tertiary',
                ]"
                :title="s.doing"
                >{{ s.doing }}</span
              >
              <span
                role="cell"
                :class="['min-w-0 truncate', dim ? 'text-tertiary' : 'text-secondary']"
                :title="s.project ?? undefined"
                >{{ s.project ?? '' }}</span
              >
              <span role="cell" class="flex min-w-0 items-center gap-1.5">
                <!-- The provider mark keeps its colour on a dimmed row: dimmed, Claude's spark would fall below 3:1. -->
                <ProviderMark :provider="s.provider" size="xs" />
                <span
                  v-if="s.signedInAs"
                  :class="['min-w-0 truncate font-medium', dim ? 'text-tertiary' : 'text-attn']"
                  :title="`Signed in as ${s.signedInAs}`"
                  >Signed in as {{ s.signedInAs }}</span
                >
                <span v-else class="min-w-0 truncate" :title="sessionAccount(s)?.label">{{
                  sessionAccount(s)?.label
                }}</span>
              </span>
              <span role="cell" class="inline-flex items-center gap-1.5 text-tertiary">
                <SubagentsBadge v-if="s.subagents.length" :label="subagentsLabel(s)" :count="s.subagents.length" />
              </span>
              <span role="cell" class="font-mono text-tertiary">{{ s.id.slice(-8) }}</span>
              <span role="cell" class="text-right text-tertiary tabular-nums">{{ runningFor(s, store.now) }}</span>
            </WTableRow>
            <div v-if="rows.length === 0" role="row">
              <span role="cell" class="block p-4 text-13 text-tertiary">
                {{
                  filter.length
                    ? 'No sessions for the checked accounts'
                    : quietCount
                      ? 'Only ended and not-known sessions, hidden'
                      : 'No sessions right now'
                }}
              </span>
            </div>
          </WTable>
        </section>
      </div>

      <section v-if="resets.length" aria-labelledby="resets-heading" class="flex min-w-0 grow basis-72 flex-col gap-3">
        <h2 id="resets-heading" class="text-16 font-semibold">Limit resets</h2>
        <WList>
          <WListItem v-for="r in resets" :key="r.key">
            <AccountIcon :account="r.account" />
            <span class="flex min-w-0 flex-1 flex-col">
              <span class="truncate font-semibold">{{ r.account.label }}</span>
              <span class="text-12 text-secondary">{{ r.title }}</span>
              <span v-if="r.soon" class="flex items-center gap-1 text-12 font-medium text-attn" data-expires-soon>
                <Hourglass aria-hidden="true" class="size-3.5 shrink-0 text-attn-icon" />
                {{ expiryText(r.expiresAt, store.now, warnDays) }}
              </span>
              <span v-else class="text-12 text-tertiary">{{ expiryText(r.expiresAt, store.now, warnDays) }}</span>
            </span>
          </WListItem>
        </WList>
      </section>
    </div>
  </div>
</template>
