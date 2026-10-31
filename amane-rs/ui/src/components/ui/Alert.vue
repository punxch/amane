<script setup lang="ts">
import { computed } from "vue";

import Icon, { type IconName } from "@/components/ui/Icon.vue";

const props = withDefaults(
  defineProps<{
    tone?: "danger" | "success" | "warning" | "info";
    title?: string;
  }>(),
  { tone: "info", title: "" },
);

const ICONS: Record<"danger" | "success" | "warning" | "info", IconName> = {
  danger: "alert",
  success: "check",
  warning: "alert",
  info: "alert",
};

const icon = computed<IconName>(() => ICONS[props.tone]);
</script>

<template>
  <div class="amane-alert" :class="`amane-alert--${props.tone}`" role="status">
    <Icon :name="icon" :size="16" class="amane-alert__icon" />
    <div class="amane-alert__body">
      <p v-if="props.title !== ''" class="amane-alert__title">{{ props.title }}</p>
      <slot />
    </div>
    <slot name="action" />
  </div>
</template>

<style scoped>
.amane-alert {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  padding: 10px 12px;
  border: 1px solid transparent;
  border-radius: var(--amane-radius);
  font-size: 14px;
  line-height: 1.5;
}

.amane-alert__icon {
  flex-shrink: 0;
  margin-top: 3px;
}

.amane-alert__body {
  flex: 1;
  min-width: 0;
}

.amane-alert__title {
  margin: 0 0 2px;
  font-weight: 600;
}

.amane-alert p {
  margin: 0;
}

.amane-alert--danger {
  background: var(--amane-error-soft);
  border-color: color-mix(in srgb, var(--amane-red) 35%, transparent);
  color: var(--amane-red);
}

.amane-alert--success {
  background: var(--amane-success-soft);
  border-color: color-mix(in srgb, var(--amane-teal) 35%, transparent);
  color: var(--amane-teal);
}

.amane-alert--warning {
  background: var(--amane-warning-soft);
  border-color: color-mix(in srgb, var(--amane-yellow) 35%, transparent);
  color: var(--amane-yellow);
}

.amane-alert--info {
  background: var(--amane-brand-soft);
  border-color: color-mix(in srgb, var(--amane-brand) 35%, transparent);
  color: var(--amane-brand-text);
}
</style>
