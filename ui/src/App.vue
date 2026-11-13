<script setup lang="ts">
// The shell: a full-height sidebar, and beside it the top strip over the content panel, which runs to the window's
// right and bottom edges and scrolls on its own. Settings and first-run setup open as modals on top.
import OverviewPage from './pages/OverviewPage.vue';
import SettingsModal from './settings/SettingsModal.vue';
import SetupModal from './settings/SetupModal.vue';
import AppSidebar from './shell/AppSidebar.vue';
import TopStrip from './shell/TopStrip.vue';
</script>

<template>
  <div class="flex h-dvh flex-wrap items-stretch bg-win text-text">
    <AppSidebar />
    <div class="flex h-full min-w-0 grow-999 basis-140 flex-col">
      <TopStrip />
      <main class="min-h-0 flex-auto overflow-y-auto rounded-tl-lg border-t border-l border-ring bg-panel px-9 py-6">
        <OverviewPage />
      </main>
    </div>
  </div>
  <SettingsModal />
  <SetupModal />
</template>
