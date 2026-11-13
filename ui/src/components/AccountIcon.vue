<script setup lang="ts">
// An account's provider mark; only fixed main has a crown.
import { Crown } from 'lucide-vue-next';
import type { Account } from '../api';
import { badgeLabel } from '../format';
import ProviderMark from './ProviderMark.vue';

const props = withDefaults(
  defineProps<{
    account: Pick<Account, 'provider' | 'id' | 'label'>;
    size?: 'md' | 'lg';
    on?: 'panel' | 'modal';
  }>(),
  { size: 'md', on: 'panel' },
);
</script>

<template>
  <span
    :class="['relative inline-flex shrink-0 items-center justify-center', props.size === 'lg' ? 'size-7' : 'size-6']"
  >
    <ProviderMark
      :provider="props.account.provider"
      :size="props.size"
      :label="props.account.provider === 'claude' ? 'Claude' : 'Codex'"
    />
    <span
      v-if="props.account.id === 'main'"
      role="img"
      :aria-label="badgeLabel(props.account)"
      :title="badgeLabel(props.account)"
      :class="[
        'absolute -right-1.5 -bottom-1.5 inline-flex h-4.25 min-w-4.25 items-center justify-center rounded-full border-2 px-1 text-10 font-bold tabular-nums',
        props.on === 'modal' ? 'border-modal' : 'border-panel',
        'bg-primary text-on-primary',
      ]"
    >
      <Crown class="size-2.25" :stroke-width="2.8" aria-hidden="true" />
    </span>
  </span>
</template>
