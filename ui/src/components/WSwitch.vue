<script setup lang="ts">
// The app's switch (Material 3 style): off is a hollow outlined track with a small knob, on is a filled track with a
// large knob, so each state reads on its own. Hovered (or its label hovered) the track lights up; pressed, it darkens a
// step further (off: a darker track, outline and knob). Nothing scales.
//
// A switch that cannot be changed says why (`disabledReason`): it stays focusable with `aria-disabled`, shows the
// reason in the shared tooltip on hover and keyboard focus, and does nothing when activated. Only the track fades,
// so the focus ring keeps its full contrast. The root is styled by `aria-checked`, not `data-state`, because the
// tooltip trigger sets its own `data-state` on it.
import { SwitchRoot, SwitchThumb } from 'reka-ui';
import { vLabelState } from './labels';
import WTooltip from './WTooltip.vue';

const model = defineModel<boolean>({ required: true });
const props = defineProps<{ label: string; disabledReason?: string }>();

const root =
  'group relative block h-6 w-10 shrink-0 cursor-pointer rounded-full focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-indicator aria-disabled:cursor-not-allowed';
const track =
  'absolute inset-0 rounded-full bg-switch-off-bg inset-ring-2 inset-ring-control group-hov:bg-chip-off group-press:bg-press-strong group-press:inset-ring-control-press group-aria-checked:bg-switch-on group-aria-checked:inset-ring-0 group-aria-checked:group-hov:bg-primary-hover group-aria-checked:group-press:bg-primary-press group-aria-disabled:opacity-40';
const thumb =
  'absolute top-1.5 left-1.5 block size-3 rounded-full bg-control data-[state=checked]:top-0.75 data-[state=checked]:left-4.75 data-[state=checked]:size-4.5 data-[state=checked]:bg-knob-on data-[state=checked]:shadow-knob data-[state=unchecked]:group-press:bg-control-press';
</script>

<template>
  <WTooltip v-if="props.disabledReason" :text="props.disabledReason" as-child>
    <SwitchRoot :model-value="model" :aria-label="props.label" aria-disabled="true" as-child>
      <button v-label-state type="button" :class="root">
        <span :class="track"><SwitchThumb :class="thumb" /></span>
      </button>
    </SwitchRoot>
  </WTooltip>
  <SwitchRoot v-else v-model="model" :aria-label="props.label" as-child>
    <button v-label-state type="button" :class="root">
      <span :class="track"><SwitchThumb :class="thumb" /></span>
    </button>
  </SwitchRoot>
</template>
