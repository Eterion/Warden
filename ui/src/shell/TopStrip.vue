<script setup lang="ts">
// The thin top strip. The status bar names the account that gets the work now and why ("Work goes to Claude 2: its week
// resets in 15 h"), or, in amber, a limit reset to use first. On hover and focus: the whole item (a narrow window cuts
// it), Warden's order of the accounts, and what main keeps for the user and why.
// Then "Usage read N min ago", which reads usage again on click. In the desktop app the strip also moves the window
// (there is no classic title bar) and ends with the window's own controls, Close in red on hover.
import { CloudOff, Copy, Hourglass, Minus, RefreshCw, Square, X } from 'lucide-vue-next';
import { computed } from 'vue';
import ProviderMark from '../components/ProviderMark.vue';
import WButton from '../components/WButton.vue';
import WTooltip from '../components/WTooltip.vue';
import { closeWindow, desktop, minimize, toggleMaximize } from '../desktop';
import { ago, workGoesTo } from '../format';
import { refreshUsage, store } from '../store';

const read = computed(() => {
  const at = store.state?.lastRead;
  return at ? `Usage read ${ago(at, store.now)}` : 'Read usage now';
});

const work = computed(() => (store.state ? workGoesTo(store.state, store.now) : null));

const caption =
  'inline-flex w-11.5 cursor-default items-center justify-center text-secondary focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-indicator';
const control = `${caption} hov:bg-active press:bg-press`;
// Close turns Windows red with a white X on hover, a darker red when pressed, as in any Windows program.
const closeControl = 'hov:bg-close-hover hov:text-on-close press:bg-close-press press:text-on-close';
</script>

<template>
  <header
    class="flex h-12 shrink-0 items-center gap-3 pl-2"
    :data-tauri-drag-region="desktop.inApp ? 'deep' : undefined"
  >
    <div role="status" class="flex min-w-0 flex-auto items-center gap-1.5 text-12 text-secondary">
      <template v-if="!store.connected">
        <CloudOff class="size-3.5 text-attn-icon" aria-hidden="true" />
        <span class="text-attn">Warden is not reachable. Showing the last state it sent.</span>
      </template>
      <WTooltip
        v-else-if="work"
        :text="work.tooltip"
        :class="['inline-flex min-w-0 items-center gap-1.5', work.attn ? 'text-attn' : '']"
        data-work
      >
        <Hourglass v-if="work.attn" class="size-3.5 shrink-0 text-attn-icon" aria-hidden="true" />
        <span class="shrink-0 whitespace-nowrap">{{ `${work.lead} ` }}</span>
        <ProviderMark :provider="work.account.provider" size="xs" />
        <!-- One flex item, so the text after the name stays with it when read out. -->
        <span class="min-w-0 truncate"
          ><span :class="['font-medium', work.attn ? '' : 'text-text']">{{ work.account.label }}</span
          >{{ work.rest }}</span
        >
        <span class="sr-only">. {{ work.order }}</span>
      </WTooltip>
    </div>
    <span v-if="store.state?.hooksOff" class="text-12 text-secondary">Hooks are off for this run</span>
    <WButton
      variant="quiet"
      size="sm"
      :icon="RefreshCw"
      :busy="store.refreshing"
      :aria-label="`${read}. Read it again`"
      class="mr-1"
      @click="refreshUsage"
    >
      {{ store.refreshing ? 'Reading usage' : read }}
    </WButton>
    <div v-if="desktop.inApp" class="flex shrink-0 self-stretch">
      <button type="button" aria-label="Minimize" title="Minimize" :class="control" @click="minimize">
        <Minus class="size-4" :stroke-width="1.6" aria-hidden="true" />
      </button>
      <button
        type="button"
        :aria-label="desktop.maximized ? 'Restore' : 'Maximize'"
        :title="desktop.maximized ? 'Restore' : 'Maximize'"
        :class="control"
        @click="toggleMaximize"
      >
        <Copy v-if="desktop.maximized" class="size-3.5 -scale-x-100" :stroke-width="1.8" aria-hidden="true" />
        <Square v-else class="size-3.5" :stroke-width="1.8" aria-hidden="true" />
      </button>
      <button type="button" aria-label="Close" title="Close" :class="[caption, closeControl]" @click="closeWindow">
        <X class="size-4" :stroke-width="1.6" aria-hidden="true" />
      </button>
    </div>
  </header>
</template>
