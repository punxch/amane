<script setup lang="ts">
import { computed } from "vue";

import HintedActionIcon from "@/components/common/HintedActionIcon.vue";
import Icon, { type IconName } from "@/components/ui/Icon.vue";
import { cycleTheme, ui } from "@/stores/ui";

/** 三态轮换: 浅色 → 深色 → 跟随系统, 说明文案给出当前态. */
const THEME_LABELS = {
  light: "主题：浅色",
  dark: "主题：深色",
  auto: "主题：跟随系统",
} as const;

const THEME_ICONS: Record<"light" | "dark" | "auto", IconName> = {
  light: "sun",
  dark: "moon",
  auto: "contrast",
};

const label = computed(() => THEME_LABELS[ui.theme]);
const icon = computed<IconName>(() => THEME_ICONS[ui.theme]);
</script>

<template>
  <HintedActionIcon :label="label" @click="cycleTheme">
    <Icon :name="icon" :size="18" />
  </HintedActionIcon>
</template>
