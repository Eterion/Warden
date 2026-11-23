<script setup lang="ts">
// Warden's line icon, used where Warden itself is meant (the Memory page's "Warden itself"): traced from the app icon
// (crates/warden-desktop/icons/warden.svg), its outline the icon's own silhouette (the head with the cap's tufts), and
// inside it only the eyes (rings with filled pupils), the beak and the brows meeting towards it.
//
// Drawn as a Lucide icon is: no fill, round caps and joins, in currentColor. It is drawn in the app icon's coordinates,
// in a square sized so the owl fills what a Lucide icon fills of its 24 grid (20 of 24), and `strokeWidth` is measured
// on that grid, so beside a Lucide icon of the same size and stroke its lines are as thick (OWL_VIEW / 24 per unit).
/** The square the owl is drawn in, in the app icon's units: its 206-unit width as 20 of 24. */
const OWL_VIEW = 247;

const props = withDefaults(defineProps<{ size?: number | string; strokeWidth?: number | string }>(), {
  size: 24,
  strokeWidth: 2,
});
</script>

<template>
  <svg
    :width="props.size"
    :height="props.size"
    :viewBox="`4.5 4.5 ${OWL_VIEW} ${OWL_VIEW}`"
    fill="none"
    stroke="currentColor"
    :stroke-width="(Number(props.strokeWidth) * OWL_VIEW) / 24"
    stroke-linecap="round"
    stroke-linejoin="round"
    data-owl-icon
  >
    <path
      d="M128 39.5C108 39.5 89 45.6 74 57.6Q73.5 58.2 72.6 57.7C60 51 47.6 41.5 36.4 31.9Q30.6 26.9 29 35.2C26.4 49 28.6 70.6 40.3 84.6C32.7 96 25 113 25 128C24.7 147.5 34.7 165.5 51.8 180C70 197 97 213 116 221.5Q124 226 128 225.6Q132 226 140 221.5C159 213 186 197 204.2 180C221.3 165.5 231.3 147.5 231 128C231 113 223.3 96 215.7 84.6C227.4 70.6 229.6 49 227 35.2Q225.4 26.9 219.6 31.9C208.4 41.5 196 51 183.4 57.7Q182.5 58.2 182 57.6C167 45.6 148 39.5 128 39.5Z"
    />
    <circle cx="88" cy="134" r="20" />
    <circle cx="168" cy="134" r="20" />
    <circle cx="88" cy="134" r="8" fill="currentColor" stroke="none" />
    <circle cx="168" cy="134" r="8" fill="currentColor" stroke="none" />
    <path
      d="M128 138C125 138 119 140.5 116 143Q111 146.5 114 151C117 157.5 121.5 164.5 126 170.5Q127 172 128 172Q129 172 130 170.5C134.5 164.5 139 157.5 142 151Q145 146.5 140 143C137 140.5 131 138 128 138Z"
    />
    <path d="M48 88C70 92 96 98 112 108Q120 113 124 121" />
    <path d="M208 88C186 92 160 98 144 108Q136 113 132 121" />
  </svg>
</template>
