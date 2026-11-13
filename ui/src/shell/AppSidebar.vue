<script setup lang="ts">
// The full-height sidebar: Warden, Overview, the accounts in use with their weekly bars, a newer Warden's download or
// "Install and restart", and Settings at the bottom. In
// the desktop app, Warden's name opens the app menu (Quit) and the row around it moves the window.
import { ChevronDown, LayoutDashboard, Power, Radar, Settings } from 'lucide-vue-next';
import { DropdownMenuPortal, DropdownMenuRoot, DropdownMenuTrigger } from 'reka-ui';
import { computed } from 'vue';
import NavItem from '../components/NavItem.vue';
import ProviderMark from '../components/ProviderMark.vue';
import StateIcon from '../components/StateIcon.vue';
import UsageBar from '../components/UsageBar.vue';
import WMenuContent from '../components/WMenuContent.vue';
import WMenuItem from '../components/WMenuItem.vue';
import { desktop, quit } from '../desktop';
import { limitView, loginIdle } from '../format';
import { openSettings } from '../route';
import { store, usedAccounts } from '../store';
import UpdateItem from './UpdateItem.vue';

const accounts = computed(() =>
  usedAccounts(store.state).map((a) => {
    const week = a.usage?.weekly ? limitView(a.usage.weekly, store.now) : null;
    const busy = (store.state?.sessions ?? []).some(
      (s) => s.accountId === a.id && !s.signedInAs && s.state === 'working' && s.source === 'hook',
    );
    return { account: a, week, busy, aria: week ? `Week ${week.percent}% used` : 'Weekly limit not read yet' };
  }),
);
</script>

<template>
  <aside class="flex max-w-60 shrink-0 grow basis-55 flex-col px-3 pb-3">
    <div v-if="desktop.inApp" class="flex h-12 shrink-0 items-center" data-tauri-drag-region="deep">
      <DropdownMenuRoot :modal="false">
        <DropdownMenuTrigger
          aria-label="Warden menu"
          class="inline-flex h-9 cursor-pointer items-center gap-2.25 rounded-lg px-2 hov:bg-active press:bg-press focus-visible:outline-2 focus-visible:outline-indicator data-[state=open]:bg-active data-[state=open]:press:bg-press"
        >
          <Radar class="size-5" aria-hidden="true" />
          <span class="text-14 font-semibold">Warden</span>
          <ChevronDown class="size-3.5 text-icon" aria-hidden="true" />
        </DropdownMenuTrigger>
        <!-- Kept inside the sidebar, so the open menu stays within its landmark. -->
        <DropdownMenuPortal disabled>
          <WMenuContent align="start">
            <WMenuItem :icon="Power" @select="quit">Quit Warden</WMenuItem>
          </WMenuContent>
        </DropdownMenuPortal>
      </DropdownMenuRoot>
    </div>
    <div v-else class="flex h-12 shrink-0 items-center gap-2.25 px-2">
      <Radar class="size-5" aria-hidden="true" />
      <span class="text-14 font-semibold">Warden</span>
    </div>

    <nav aria-label="Main" class="flex flex-col gap-0.5 pt-2">
      <NavItem href="#" :icon="LayoutDashboard" current>Overview</NavItem>
    </nav>

    <h2 class="px-2 pt-5 pb-1 text-12 font-medium text-secondary">Accounts</h2>
    <ul class="flex flex-col gap-0.5">
      <li
        v-for="{ account, week, busy, aria } in accounts"
        :key="account.id"
        class="flex min-h-8.5 items-center gap-3 rounded-lg px-2 text-14 font-medium"
      >
        <span class="inline-flex size-4.5 shrink-0 items-center justify-center">
          <ProviderMark
            :provider="account.provider"
            size="sm"
            :label="account.provider === 'claude' ? 'Claude' : 'Codex'"
          />
        </span>
        <span class="min-w-0 flex-auto truncate">{{ account.label }}</span>
        <span v-if="busy" role="img" aria-label="Working" title="Working" class="inline-flex">
          <StateIcon state="working" size="sm" />
        </span>
        <span role="img" :aria-label="aria" :title="aria" class="shrink-0">
          <UsageBar
            :percent="week?.percent ?? 0"
            :level="loginIdle(account) ? 'idle' : (week?.level ?? 'low')"
            track="win"
          />
        </span>
      </li>
    </ul>

    <div class="mt-auto flex flex-col gap-0.5">
      <UpdateItem />
      <NavItem :icon="Settings" @click="openSettings()">Settings</NavItem>
    </div>
  </aside>
</template>
