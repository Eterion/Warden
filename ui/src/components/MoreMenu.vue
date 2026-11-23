<script setup lang="ts">
// The "more actions" menu on a row. Each item has a Lucide icon; destructive ones (`danger`) are red. Inside a
// modal the menu is drawn into it.
import { Ellipsis } from '@lucide/vue';
import { DropdownMenuPortal, DropdownMenuRoot, DropdownMenuTrigger } from 'reka-ui';
import { useTemplateRef, type Component } from 'vue';
import { usePortalTarget } from './portal';
import WIconButton from './WIconButton.vue';
import WMenuContent from './WMenuContent.vue';
import WMenuItem from './WMenuItem.vue';

export interface MenuItem {
  label: string;
  icon: Component;
  run: () => void;
  danger?: boolean;
  disabled?: boolean;
}

const props = defineProps<{ label: string; items: MenuItem[] }>();
const trigger = useTemplateRef<{ $el: Node | null }>('trigger');
const to = usePortalTarget(trigger);
</script>

<template>
  <DropdownMenuRoot :modal="false">
    <DropdownMenuTrigger ref="trigger" as-child>
      <WIconButton :label="props.label" :icon="Ellipsis" size="sm" />
    </DropdownMenuTrigger>
    <DropdownMenuPortal :to="to">
      <WMenuContent>
        <WMenuItem
          v-for="item in props.items"
          :key="item.label"
          :icon="item.icon"
          :danger="item.danger"
          :disabled="item.disabled"
          @select="item.run"
        >
          {{ item.label }}
        </WMenuItem>
      </WMenuContent>
    </DropdownMenuPortal>
  </DropdownMenuRoot>
</template>
