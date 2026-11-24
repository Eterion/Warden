<script setup lang="ts">
// The shell: a full-height sidebar, and beside it the top strip over the content panel (the Overview, or the Memory
// page at #/memory), which runs to the window's
// right and bottom edges and scrolls on its own. Settings and first-run setup open as modals on top.
// The shell is exactly the window's height and never wraps, and the sidebar and the panel each scroll what does not
// fit, so the page itself never scrolls. The panel is `relative`: what is positioned inside it (screen-reader-only
// text in a long table) is then clipped by its scrolling, instead of reaching below the window and making the page
// taller. The page sits in a box of its own because its tooltips and menus are drawn beside that box into <main>.
// Vue replaces the page within the box, clear of those portals, and the old page fades over the new one's layout.
import { usePreferredReducedMotion } from '@vueuse/core';
import MemoryPage from './pages/MemoryPage.vue';
import OverviewPage from './pages/OverviewPage.vue';
import { route } from './route';
import SettingsModal from './settings/SettingsModal.vue';
import SetupModal from './settings/SetupModal.vue';
import AppSidebar from './shell/AppSidebar.vue';
import TopStrip from './shell/TopStrip.vue';
import { store } from './store';

const motion = usePreferredReducedMotion();

function leavePage(page: Element): void {
  // Keep the departing page out of the tab order and accessibility tree during the cross-fade.
  page.setAttribute('inert', '');
}
</script>

<template>
  <div class="flex h-dvh items-stretch bg-win text-text">
    <AppSidebar />
    <div class="flex h-full min-w-0 grow-999 basis-140 flex-col">
      <TopStrip />
      <main
        class="relative min-h-0 flex-auto overflow-y-auto rounded-tl-lg border-t border-l border-ring bg-panel px-9 py-6"
      >
        <div class="page-content relative min-h-full">
          <Transition
            :name="motion === 'reduce' ? 'page-fade' : 'page'"
            :css="!store.away && (store.state?.settings.pageTransitions ?? true)"
            @before-leave="leavePage"
          >
            <MemoryPage v-if="route.page.value === 'memory'" />
            <OverviewPage v-else />
          </Transition>
        </div>
      </main>
    </div>
  </div>
  <SettingsModal />
  <SetupModal />
</template>
