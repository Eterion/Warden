<script setup lang="ts">
// The memory chart's drawing, in SVG (no chart library), shared by the Memory page's chart (MemoryChart) and the
// Overview's small one (MemoryCard): the minutes Warden was not running as an empty band, named on one line or two
// where that fits; faint gridlines labelled in GB; a stacked band per account (coloured by provider, greys for the
// groups, the `hovered` band in chart-hover), each edged by a thin line in the panel's colour; the baseline with its
// ticks and times; and the guide's line at `guideX` when there is one. Decoration only: whoever holds it says what it
// shows. `width` is the plot's, beside the gutter; `chart` its size.
import { computed } from 'vue';
import { MISSING_TEXT, bandPaths, type Axis, type BandColour, type ChartModel, type ChartSize } from '../memory';

const props = defineProps<{
  model: ChartModel;
  width: number;
  chart: ChartSize;
  axis: Axis;
  hovered: string | null;
  guideX: number | null;
}>();

const FILL: Record<BandColour, string> = {
  'claude-1': 'fill-chart-claude-1',
  'claude-2': 'fill-chart-claude-2',
  'claude-3': 'fill-chart-claude-3',
  'codex-1': 'fill-chart-codex-1',
  'codex-2': 'fill-chart-codex-2',
  'codex-3': 'fill-chart-codex-3',
  'other-sign-in': 'fill-chart-other-sign-in',
  left: 'fill-chart-left',
};
const ANCHOR = { start: 'start', end: 'end', middle: 'middle' } as const;

const span = computed(() => props.model.to - props.model.from);
const x = (m: number) => props.chart.gutter + ((m - props.model.from) / span.value) * props.width;
const y = (kib: number) => props.chart.top + (1 - kib / props.model.top) * (props.chart.height - props.chart.top);

const paths = computed(() => bandPaths(props.model, props.width, props.chart));
/** A gap's label: on one line where it fits, on two ("Warden", "not running") in a narrower gap, none in a sliver. */
const GAP_ONE_LINE = 120;
const GAP_TWO_LINES = 76;
const gaps = computed(() =>
  props.model.gaps
    .map((g) => {
      const w = x(g.to) - x(g.from);
      const lines =
        w >= GAP_ONE_LINE ? [MISSING_TEXT['not-running']] : w >= GAP_TWO_LINES ? ['Warden', 'not running'] : [];
      return { ...g, x: x(g.from), w, lines };
    })
    .filter((g) => g.w > 0),
);
/** The plot's middle, where a gap's label sits. */
const middle = computed(() => props.chart.top + (props.chart.height - props.chart.top) / 2);
</script>

<template>
  <svg
    :width="props.chart.gutter + props.width"
    :height="props.chart.height + props.chart.axis"
    class="block overflow-visible text-12"
    aria-hidden="true"
  >
    <!-- The plot on the panel's colour, so its bands keep their contrast where the chart sits on a tint (the
         Overview's small chart, lit as a link). -->
    <rect
      :x="props.chart.gutter"
      :y="0"
      :width="props.width"
      :height="props.chart.height"
      class="fill-panel"
      data-plot
    />
    <!-- Hours Warden was not running: an empty band, named where it fits. -->
    <g v-for="g in gaps" :key="g.from">
      <rect
        :x="g.x"
        :y="props.chart.top"
        :width="g.w"
        :height="props.chart.height - props.chart.top"
        class="fill-chart-gap"
        data-gap
      />
      <text
        v-if="g.lines.length"
        :x="g.x + g.w / 2"
        :y="middle"
        text-anchor="middle"
        dominant-baseline="middle"
        class="fill-tertiary"
        data-not-running
      >
        <tspan
          v-for="(line, i) in g.lines"
          :key="line"
          :x="g.x + g.w / 2"
          :dy="i === 0 ? (-(g.lines.length - 1) * props.chart.line) / 2 : props.chart.line"
        >
          {{ line }}
        </tspan>
      </text>
    </g>
    <!-- Faint gridlines, labelled in GB. -->
    <g v-for="line in props.model.grid" :key="line.kib" data-gridline>
      <line
        :x1="props.chart.gutter"
        :x2="props.chart.gutter + props.width"
        :y1="y(line.kib)"
        :y2="y(line.kib)"
        class="stroke-divider-strong"
        stroke-dasharray="2 3"
      />
      <text
        :x="props.chart.gutter - 8"
        :y="y(line.kib)"
        text-anchor="end"
        dominant-baseline="middle"
        class="fill-tertiary"
      >
        {{ line.label }}
      </text>
    </g>
    <!-- The bands, bottom first, each edged on top by a thin line in the panel's colour. -->
    <g :transform="`translate(${props.chart.gutter} 0)`">
      <path
        v-for="p in paths"
        :key="p.key"
        :d="p.area"
        :class="props.hovered === p.key ? 'fill-chart-hover' : FILL[p.colour]"
        :data-band="p.key"
      />
      <path
        v-for="p in paths"
        :key="`${p.key}-edge`"
        :d="p.edge"
        class="fill-none stroke-panel"
        stroke-width="1"
        stroke-linejoin="round"
      />
    </g>
    <!-- The baseline over the bands' feet, its ticks (longer under a label) and the times. -->
    <g data-axis>
      <line
        :x1="props.chart.gutter"
        :x2="props.chart.gutter + props.width"
        :y1="props.chart.height + 0.5"
        :y2="props.chart.height + 0.5"
        class="stroke-axis"
      />
      <line
        v-for="t in props.axis.ticks"
        :key="`tick-${t.m}`"
        :x1="props.chart.gutter + t.x"
        :x2="props.chart.gutter + t.x"
        :y1="props.chart.height + 0.5"
        :y2="props.chart.height + (t.major ? props.chart.tick : props.chart.tick / 2)"
        class="stroke-axis"
        :data-tick="t.major ? 'major' : 'minor'"
      />
      <text
        v-for="t in props.axis.labels"
        :key="t.m"
        :x="props.chart.gutter + t.x"
        :y="props.chart.height + props.chart.axis - 2"
        :text-anchor="ANCHOR[t.anchor ?? 'middle']"
        class="fill-tertiary"
        data-axis-label
      >
        {{ t.label }}
      </text>
    </g>
    <!-- The guide: a line in the text colour on a wider one in the panel's, so it stands out on every band. -->
    <g v-if="props.guideX !== null" data-guide>
      <line
        :x1="props.guideX"
        :x2="props.guideX"
        :y1="props.chart.top"
        :y2="props.chart.height"
        class="stroke-panel"
        stroke-width="3"
      />
      <line
        :x1="props.guideX"
        :x2="props.guideX"
        :y1="props.chart.top"
        :y2="props.chart.height"
        class="stroke-text"
        stroke-width="1"
      />
    </g>
  </svg>
</template>
