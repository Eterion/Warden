<script setup lang="ts">
// The Memory page's day: one button (a calendar icon, the day, a small chevron) opening a month calendar in a popover,
// built from Reka UI's Calendar inside its Popover. Days with recordings carry a small neutral dot under the number; the
// day shown is filled, today (the page's, as Previous and Next count it) has a ring, and days after today are disabled. The month steps back freely and forward
// no further than this one. Picking a day goes to its address and closes the popover, and focus goes back to the
// button. The keyboard is Reka's: arrows between days, Page Up and Page Down between months, Enter to pick.
import { parseDate, type DateValue } from '@internationalized/date';
import { CalendarDays, ChevronDown, ChevronLeft, ChevronRight } from '@lucide/vue';
import {
  CalendarCell,
  CalendarCellTrigger,
  CalendarGrid,
  CalendarGridBody,
  CalendarGridHead,
  CalendarGridRow,
  CalendarHeadCell,
  CalendarHeading,
  CalendarRoot,
  PopoverContent,
  PopoverPortal,
  PopoverRoot,
  PopoverTrigger,
} from 'reka-ui';
import { computed, ref, shallowRef, useTemplateRef, watch } from 'vue';
import { usePortalTarget } from '../components/portal';
import WIconButton from '../components/WIconButton.vue';
import { dayTitle } from '../memory';

const props = defineProps<{
  /** The day shown (yyyy-MM-dd). */
  day: string;
  today: string;
  /** The days with recordings (yyyy-MM-dd), as `MemoryDay` lists them. */
  recorded: string[];
}>();
const emit = defineEmits<{ pick: [day: string] }>();

const open = ref(false);
const chosen = computed(() => parseDate(props.day));
const max = computed(() => parseDate(props.today));
/** The month on view: the day shown's whenever the popover opens. */
const month = shallowRef<DateValue>(parseDate(props.day));
watch(open, (o) => {
  if (o) month.value = parseDate(props.day);
});
const recorded = computed(() => new Set(props.recorded));
const atLatest = computed(() => month.value.year === max.value.year && month.value.month === max.value.month);

function step(months: number): void {
  month.value = month.value.add({ months }).set({ day: 1 });
}

/** A day picked (or the one shown picked again, which Reka passes on as none): go there, and close. */
function pick(value: DateValue | undefined): void {
  open.value = false;
  if (value && value.compare(max.value) <= 0) emit('pick', value.toString());
}

/** A day's name for assistive technology, as the page writes days, and whether it has recordings. */
function label(d: DateValue): string {
  const iso = d.toString();
  return `${dayTitle(iso, props.today)}${recorded.value.has(iso) ? ', recorded' : ''}`;
}

const trigger = useTemplateRef<{ $el: Node | null }>('trigger');
const to = usePortalTarget(trigger);

/** Today's ring: in the indicator colour, or, on the filled day shown, in the fill's text colour inside its edge. */
const TODAY = 'inset-ring-1 inset-ring-indicator data-[selected]:inset-ring-0 data-[selected]:shadow-today-selected';
const CELL =
  'relative inline-flex size-9 cursor-pointer flex-col items-center justify-center rounded-lg text-13 tabular-nums text-text outline-none hov:bg-chip-off press:bg-press focus-visible:outline-2 focus-visible:outline-indicator data-[selected]:bg-primary data-[selected]:font-semibold data-[selected]:text-on-primary data-[selected]:hov:bg-primary-hover data-[selected]:press:bg-primary-press data-[disabled]:cursor-not-allowed data-[disabled]:opacity-40 data-[outside-view]:invisible';
</script>

<template>
  <PopoverRoot v-model:open="open">
    <PopoverTrigger ref="trigger" as-child>
      <button
        type="button"
        class="inline-flex h-8 shrink-0 cursor-pointer items-center justify-center gap-1.5 rounded-lg px-2 text-center text-14 font-semibold whitespace-nowrap text-text hov:bg-chip-off press:bg-press focus-visible:outline-2 focus-visible:outline-indicator data-[state=open]:bg-chip-off data-[state=open]:press:bg-press"
        data-day-picker
      >
        <CalendarDays class="size-4 text-icon" aria-hidden="true" />
        <span data-day>{{ dayTitle(props.day, props.today) }}</span>
        <ChevronDown class="size-3.5 text-icon" aria-hidden="true" />
      </button>
    </PopoverTrigger>
    <PopoverPortal :to="to">
      <PopoverContent
        align="center"
        :side-offset="4"
        :collision-padding="8"
        aria-label="Choose a day"
        class="z-50 origin-popover rounded-lg bg-modal p-3 text-text shadow-modal data-[state=closed]:animate-menu-out data-[state=open]:animate-menu-in"
        data-calendar
      >
        <CalendarRoot
          v-slot="{ grid, weekDays }"
          :model-value="chosen"
          :placeholder="month"
          :max-value="max"
          locale="en"
          :week-starts-on="1"
          weekday-format="short"
          fixed-weeks
          prevent-deselect
          initial-focus
          @update:placeholder="(p: DateValue) => (month = p)"
          @update:model-value="pick"
        >
          <div class="mb-2 flex items-center justify-between gap-2">
            <WIconButton size="sm" label="Previous month" :icon="ChevronLeft" @click="step(-1)" />
            <CalendarHeading class="text-14 font-semibold" data-month />
            <WIconButton
              size="sm"
              label="Next month"
              :icon="ChevronRight"
              :disabled-reason="atLatest ? 'This is the current month.' : null"
              @click="!atLatest && step(1)"
            />
          </div>
          <CalendarGrid v-for="m in grid" :key="m.value.toString()" class="border-collapse">
            <CalendarGridHead>
              <CalendarGridRow class="grid grid-cols-7">
                <CalendarHeadCell v-for="d in weekDays" :key="d" class="pb-1 text-12 font-normal text-secondary">
                  {{ d }}
                </CalendarHeadCell>
              </CalendarGridRow>
            </CalendarGridHead>
            <CalendarGridBody>
              <CalendarGridRow v-for="(week, i) in m.rows" :key="i" class="grid grid-cols-7">
                <CalendarCell v-for="d in week" :key="d.toString()" :date="d" class="p-0">
                  <CalendarCellTrigger
                    :day="d"
                    :month="m.value"
                    :class="[CELL, d.toString() === props.today && TODAY]"
                    :aria-label="label(d)"
                    :aria-current="d.toString() === props.today ? 'date' : undefined"
                  >
                    {{ d.day }}
                    <span
                      v-if="recorded.has(d.toString())"
                      aria-hidden="true"
                      class="absolute bottom-1 size-1 rounded-full bg-icon in-data-[selected]:bg-on-primary"
                      data-recorded
                    ></span>
                  </CalendarCellTrigger>
                </CalendarCell>
              </CalendarGridRow>
            </CalendarGridBody>
          </CalendarGrid>
        </CalendarRoot>
      </PopoverContent>
    </PopoverPortal>
  </PopoverRoot>
</template>
