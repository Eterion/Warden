<script setup lang="ts">
// The shell: a full-height sidebar, and beside it the top strip over the content panel, which runs to the window's
// right and bottom edges and scrolls on its own. Settings and first-run setup open as modals on top.
// The shell is exactly the window's height and never wraps, and the sidebar and the panel each scroll what does not
// fit, so the page itself never scrolls. The panel is `relative`: what is positioned inside it (screen-reader-only
// text in a long table) is then clipped by its scrolling, instead of reaching below the window and making the page
// taller.
import OverviewPage from './pages/OverviewPage.vue';
import SettingsModal from './settings/SettingsModal.vue';
import SetupModal from './settings/SetupModal.vue';
import AppSidebar from './shell/AppSidebar.vue';
import TopStrip from './shell/TopStrip.vue';
</script>

<template>
  <div class="flex h-dvh items-stretch bg-win text-text">
    <AppSidebar />
    <div class="flex h-full min-w-0 grow-999 basis-140 flex-col">
      <TopStrip />
      <main
        class="relative min-h-0 flex-auto overflow-y-auto rounded-tl-lg border-t border-l border-ring bg-panel px-9 py-6"
      >
        <OverviewPage />
      </main>
    </div>
  </div>
  <SettingsModal />
  <SetupModal />
</template>
