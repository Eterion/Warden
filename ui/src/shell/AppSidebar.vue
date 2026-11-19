<script setup lang="ts">
// The full-height sidebar: Warden, Overview, the accounts in use with their weekly bars (and Add account beside their
// heading), a newer Warden's download or "Install and restart", and Settings at the bottom. In
// the desktop app, Warden's name opens the app menu (Settings, Accounts, Quit) and the row around it moves the window.
// In a window too short for all of it, the sidebar scrolls on its own (`relative`, so nothing positioned inside reaches
// past it).
import { ChevronDown, CircleUserRound, LayoutDashboard, Plus, Power, Settings } from 'lucide-vue-next';
import { DropdownMenuPortal, DropdownMenuRoot, DropdownMenuSeparator, DropdownMenuTrigger } from 'reka-ui';
import { computed, useTemplateRef } from 'vue';
import NavItem from '../components/NavItem.vue';
import ProviderMark from '../components/ProviderMark.vue';
import StateIcon from '../components/StateIcon.vue';
import UsageBar from '../components/UsageBar.vue';
import WIconButton from '../components/WIconButton.vue';
import WMenuContent from '../components/WMenuContent.vue';
import WMenuItem from '../components/WMenuItem.vue';
import { desktop, quit } from '../desktop';
import { limitView, loginIdle } from '../format';
import { openAddAccount, openSettings, route, type Section } from '../route';
import { store, usedAccounts } from '../store';
import UpdateItem from './UpdateItem.vue';
// Warden's own mark, the owl: the app icon's own source, so the two never differ.
import mark from '../../../crates/warden-desktop/icons/warden.svg';

const accounts = computed(() =>
  usedAccounts(store.state).map((a) => {
    const week = a.usage?.weekly ? limitView(a.usage.weekly, store.now) : null;
    const busy = (store.state?.sessions ?? []).some(
      (s) => s.accountId === a.id && !s.signedInAs && s.state === 'working' && s.source === 'hook',
    );
    return { account: a, week, busy, aria: week ? `Week ${week.percent}% used` : 'Weekly limit not read yet' };
  }),
);

const menuButton = useTemplateRef<{ $el: HTMLElement }>('menuButton');
/** The Settings section a menu item asked for, opened once the menu has closed. */
let pending: Section | null = null;

/**
 * Asks for Settings in a section from the Warden menu. It opens once the menu has closed (`onMenuClosed`): while the
 * menu animates closed it would give focus back to its button after Settings had taken it.
 */
function openFromMenu(section: Section): void {
  pending = section;
}

/** Counts the menu's openings, so a section asked for before the latest one is dropped. */
let openings = 0;

/** Opening the menu again forgets a section asked for before (one chosen as it was still closing). */
function onMenuOpen(open: boolean): void {
  if (!open) return;
  pending = null;
  openings++;
}

/**
 * The menu has closed. Reka gives focus back to its button on a timer it starts with this same event (after this
 * handler, so on the turn after this one's first); Settings opens on the turn after that, so nothing takes focus from it
 * afterwards. Settings gives focus back to the button as it closes (the item it was opened from is gone). Anything
 * newer wins: Settings or another dialog already open (the sidebar's Settings, a reset's confirmation), or the menu
 * open again.
 */
function onMenuClosed(): void {
  const section = pending;
  pending = null;
  if (!section) return;
  const button = menuButton.value?.$el ?? null;
  const asked = openings;
  setTimeout(() =>
    setTimeout(() => {
      const busy = route.settingsOpen.value || document.querySelector('[role=dialog]');
      if (asked === openings && !busy) openSettings(section, () => button);
    }),
  );
}
</script>

<template>
  <aside class="relative flex min-h-0 max-w-60 shrink-0 grow basis-55 flex-col overflow-y-auto px-3 pb-3">
    <div v-if="desktop.inApp" class="flex h-12 shrink-0 items-center" data-tauri-drag-region="deep">
      <DropdownMenuRoot :modal="false" @update:open="onMenuOpen">
        <DropdownMenuTrigger
          ref="menuButton"
          aria-label="Warden menu"
          class="inline-flex h-9 cursor-pointer items-center gap-2.25 rounded-lg px-2 hov:bg-active press:bg-press focus-visible:outline-2 focus-visible:outline-indicator data-[state=open]:bg-active data-[state=open]:press:bg-press"
        >
          <img :src="mark" alt="" class="size-5" />
          <span class="text-14 font-semibold">Warden</span>
          <ChevronDown class="size-3.5 text-icon" aria-hidden="true" />
        </DropdownMenuTrigger>
        <!-- Kept inside the sidebar, so the open menu stays within its landmark. -->
        <DropdownMenuPortal disabled>
          <WMenuContent align="start" @close-auto-focus="onMenuClosed">
            <WMenuItem :icon="Settings" @select="openFromMenu('general')">Settings</WMenuItem>
            <WMenuItem :icon="CircleUserRound" @select="openFromMenu('accounts')">Accounts</WMenuItem>
            <DropdownMenuSeparator class="-mx-1 my-1 h-px bg-divider-strong" />
            <WMenuItem :icon="Power" @select="quit">Quit Warden</WMenuItem>
          </WMenuContent>
        </DropdownMenuPortal>
      </DropdownMenuRoot>
    </div>
    <div v-else class="flex h-12 shrink-0 items-center gap-2.25 px-2">
      <img :src="mark" alt="" class="size-5" />
      <span class="text-14 font-semibold">Warden</span>
    </div>

    <nav aria-label="Main" class="flex flex-col gap-0.5 pt-2">
      <NavItem href="#" :icon="LayoutDashboard" current>Overview</NavItem>
    </nav>

    <div class="flex items-center justify-between gap-2 pt-4 pb-0.5 pl-2">
      <h2 class="text-12 font-medium text-secondary">Accounts</h2>
      <WIconButton label="Add account" :icon="Plus" size="xs" @click="openAddAccount" />
    </div>
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
