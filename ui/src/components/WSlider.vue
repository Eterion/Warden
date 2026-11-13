<script setup lang="ts">
// The app's slider: a thin track with a dark fill up to a round thumb. The value is shown beside it by the caller.
// `labelledby` names a visible label instead of `label`: hovering or pressing it shows the thumb's own states.
// Hovered, the thumb greys a little; pressed, it darkens further (nothing scales). It reports every move (v-model) and the value it was let go at (`commit`), so a caller can save once per drag.
// `range` gives two thumbs (a start and an end, `thumbLabels` naming each), the model then a pair; they never cross.
//
// A slider that cannot be changed says why (`disabledReason`): its thumb stays focusable with `aria-disabled`, shows
// the reason in the shared tooltip on hover and keyboard focus, and ignores drags and keys. Only the knob fades, so
// the focus ring keeps its full contrast.
import { SliderRange, SliderRoot, SliderThumb, SliderTrack } from 'reka-ui';
import { computed } from 'vue';
import { vLabelState } from './labels';
import WTooltip from './WTooltip.vue';

const model = defineModel<number | number[]>({ required: true });
const props = withDefaults(
  defineProps<{
    label?: string;
    labelledby?: string;
    min: number;
    max: number;
    step?: number;
    valueText?: string;
    disabledReason?: string;
    range?: boolean;
    /** With `range`: each thumb's name and spoken value. */
    thumbLabels?: string[];
    valueTexts?: string[];
  }>(),
  {
    label: undefined,
    labelledby: undefined,
    step: 1,
    valueText: undefined,
    disabledReason: undefined,
    range: false,
    thumbLabels: undefined,
    valueTexts: undefined,
  },
);
const emit = defineEmits<{ commit: [value: number | number[]] }>();

const locked = computed(() => props.disabledReason !== undefined);

const values = computed({
  get: () => (Array.isArray(model.value) ? model.value : [model.value]),
  set: (v: number[]) => {
    if (locked.value || v[0] === undefined) return;
    model.value = props.range ? [...v] : v[0];
  },
});

function onCommit(v: number[]): void {
  if (!locked.value && v[0] !== undefined) emit('commit', props.range ? [...v] : v[0]);
}

/** Each thumb's name: the slider's own, or with `range` the thumb's. */
function thumbLabel(k: number): string | undefined {
  return props.range ? (props.thumbLabels?.[k] ?? props.label) : props.label;
}

function thumbText(k: number): string | undefined {
  return props.range ? props.valueTexts?.[k] : props.valueText;
}

const THUMB =
  'group block size-4 rounded-full focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-indicator';
</script>

<template>
  <SliderRoot
    v-model="values"
    :min="props.min"
    :max="props.max"
    :step="props.step"
    :min-steps-between-thumbs="props.range ? 1 : 0"
    :class="[
      'relative flex h-5 w-full min-w-20 touch-none items-center select-none',
      locked ? 'cursor-not-allowed' : 'cursor-pointer',
    ]"
    @value-commit="onCommit"
  >
    <SliderTrack class="relative h-1 grow rounded-full bg-track">
      <SliderRange :class="['absolute h-full rounded-full', locked ? 'bg-control' : 'bg-fill']" />
    </SliderTrack>
    <template v-if="props.disabledReason">
      <WTooltip v-for="(_, k) in values" :key="k" :text="props.disabledReason" as-child>
        <SliderThumb
          :aria-label="thumbLabel(k)"
          :aria-labelledby="props.range ? undefined : props.labelledby"
          :aria-valuetext="thumbText(k)"
          aria-disabled="true"
          :class="THUMB"
        >
          <span class="block size-4 rounded-full bg-thumb opacity-60 shadow-thumb" />
        </SliderThumb>
      </WTooltip>
    </template>
    <template v-else>
      <SliderThumb
        v-for="(_, k) in values"
        :key="k"
        v-label-state
        :aria-label="thumbLabel(k)"
        :aria-labelledby="props.range ? undefined : props.labelledby"
        :aria-valuetext="thumbText(k)"
        :class="THUMB"
      >
        <span
          class="block size-4 rounded-full bg-thumb shadow-thumb group-hov:bg-thumb-hover group-press:bg-thumb-press"
        />
      </SliderThumb>
    </template>
  </SliderRoot>
</template>
