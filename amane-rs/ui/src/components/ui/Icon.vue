<script setup lang="ts">
/**
 * 内联图标.
 *
 * 线条风格与原项目使用的 Tabler 图标同形 (24 格、1.6 描边、圆头), 但不引入图标库:
 * 用到的图标固定且很少, 内联一份比多一个依赖更省.
 */
import { computed } from "vue";

const ICON_PATHS = {
  search: "M10 10m-7 0a7 7 0 1 0 14 0a7 7 0 1 0 -14 0M21 21l-6 -6",
  sun: "M12 12m-4 0a4 4 0 1 0 8 0a4 4 0 1 0 -8 0M12 3v1M12 20v1M3 12h1M20 12h1M5.6 5.6l.7 .7M17.7 17.7l.7 .7M5.6 18.4l.7 -.7M17.7 6.3l.7 -.7",
  moon: "M12 3a9 9 0 1 0 9 9a7 7 0 0 1 -9 -9z",
  contrast:
    "M12 12m-9 0a9 9 0 1 0 18 0a9 9 0 1 0 -18 0M12 3v18M12 3a9 9 0 0 1 0 18",
  github:
    "M9 19c-4.3 1.4 -4.3 -2.5 -6 -3m12 5v-3.5c0 -1 .1 -1.4 -.5 -2c2.8 -.3 5.5 -1.4 5.5 -6a4.6 4.6 0 0 0 -1.3 -3.2a4.2 4.2 0 0 0 -.1 -3.2s-1.1 -.3 -3.5 1.3a12.3 12.3 0 0 0 -6.2 0c-2.4 -1.6 -3.5 -1.3 -3.5 -1.3a4.2 4.2 0 0 0 -.1 3.2a4.6 4.6 0 0 0 -1.3 3.2c0 4.6 2.7 5.7 5.5 6c-.6 .6 -.6 1.2 -.5 2v3.5",
  menu: "M4 6h16M4 12h16M4 18h16",
  collapse: "M4 4h16v16h-16zM9 4v16M15 10l-2 2l2 2",
  expand: "M4 4h16v16h-16zM9 4v16M14 10l2 2l-2 2",
  refresh:
    "M20 11a8 8 0 0 0 -8 -8a8 8 0 0 0 -8 8M4 13a8 8 0 0 0 8 8a8 8 0 0 0 8 -8M20 4v7h-7M4 20v-7h7",
  plus: "M12 5v14M5 12h14",
  close: "M18 6l-12 12M6 6l12 12",
  trash:
    "M4 7h16M10 11v6M14 11v6M5 7l1 12a2 2 0 0 0 2 2h8a2 2 0 0 0 2 -2l1 -12M9 7v-3a1 1 0 0 1 1 -1h4a1 1 0 0 1 1 1v3",
  external:
    "M12 6h-6a2 2 0 0 0 -2 2v10a2 2 0 0 0 2 2h10a2 2 0 0 0 2 -2v-6M11 13l9 -9M15 4h5v5",
  filter: "M4 5h16l-6 7v6l-4 2v-8z",
  "chevron-left": "M15 6l-6 6l6 6",
  "chevron-right": "M9 6l6 6l-6 6",
  lock: "M5 13a2 2 0 0 1 2 -2h10a2 2 0 0 1 2 2v6a2 2 0 0 1 -2 2h-10a2 2 0 0 1 -2 -2zM11 16a1 1 0 1 0 2 0a1 1 0 0 0 -2 0M8 11v-4a4 4 0 1 1 8 0v4",
  unlock:
    "M5 13a2 2 0 0 1 2 -2h10a2 2 0 0 1 2 2v6a2 2 0 0 1 -2 2h-10a2 2 0 0 1 -2 -2zM11 16a1 1 0 1 0 2 0a1 1 0 0 0 -2 0M8 11v-4a4 4 0 0 1 8 0",
  pencil: "M4 20h4l10.5 -10.5a2.828 2.828 0 1 0 -4 -4l-10.5 10.5v4M13.5 6.5l4 4",
  movie: "M4 4h16v16h-16zM4 9h16M4 15h16M9 4v5M15 4v5M9 15v5M15 15v5",
  folder:
    "M5 4h4l3 3h7a2 2 0 0 1 2 2v8a2 2 0 0 1 -2 2h-14a2 2 0 0 1 -2 -2v-11a2 2 0 0 1 2 -2",
  list: "M9 6h11M9 12h11M9 18h11M4 6h.01M4 12h.01M4 18h.01",
  clock: "M12 12m-9 0a9 9 0 1 0 18 0a9 9 0 1 0 -18 0M12 7v5l3 2",
  settings:
    "M12 12m-3 0a3 3 0 1 0 6 0a3 3 0 1 0 -6 0M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06 .06a2 2 0 1 1 -2.83 2.83l-.06 -.06a1.65 1.65 0 0 0 -1.82 -.33a1.65 1.65 0 0 0 -1 1.51v.17a2 2 0 1 1 -4 0v-.09a1.65 1.65 0 0 0 -1.08 -1.51a1.65 1.65 0 0 0 -1.82 .33l-.06 .06a2 2 0 1 1 -2.83 -2.83l.06 -.06a1.65 1.65 0 0 0 .33 -1.82a1.65 1.65 0 0 0 -1.51 -1h-.17a2 2 0 1 1 0 -4h.09a1.65 1.65 0 0 0 1.51 -1.08a1.65 1.65 0 0 0 -.33 -1.82l-.06 -.06a2 2 0 1 1 2.83 -2.83l.06 .06a1.65 1.65 0 0 0 1.82 .33h.09a1.65 1.65 0 0 0 1 -1.51v-.17a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51a1.65 1.65 0 0 0 1.82 -.33l.06 -.06a2 2 0 1 1 2.83 2.83l-.06 .06a1.65 1.65 0 0 0 -.33 1.82v.09a1.65 1.65 0 0 0 1.51 1h.17a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0 -1.51 1z",
  alert: "M12 12m-9 0a9 9 0 1 0 18 0a9 9 0 1 0 -18 0M12 8v4M12 16h.01",
  check: "M5 12l5 5l9 -9",
  arrow: "M5 12h14M13 6l6 6l-6 6",
} as const;

export type IconName = keyof typeof ICON_PATHS;

const props = withDefaults(defineProps<{ name: IconName; size?: number; stroke?: number }>(), {
  size: 18,
  stroke: 1.6,
});

const path = computed(() => ICON_PATHS[props.name]);
</script>

<template>
  <svg
    :width="props.size"
    :height="props.size"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    :stroke-width="props.stroke"
    stroke-linecap="round"
    stroke-linejoin="round"
    aria-hidden="true"
    focusable="false"
  >
    <path :d="path" />
  </svg>
</template>
