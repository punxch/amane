<script setup lang="ts">
import { computed } from "vue";

const props = withDefaults(
  defineProps<{
    tone?: "brand" | "success" | "warning" | "danger" | "gray";
    variant?: "light" | "filled" | "outline";
    size?: "sm" | "md";
  }>(),
  { tone: "gray", variant: "light", size: "md" },
);

const classes = computed(() => [
  "amane-badge",
  `amane-badge--${props.tone}`,
  `amane-badge--${props.variant}`,
  props.size === "sm" ? "amane-badge--sm" : "",
]);
</script>

<template>
  <span :class="classes"><slot /></span>
</template>

<style scoped>
.amane-badge {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 0 8px;
  border: 1px solid transparent;
  border-radius: 4px;
  font-size: 12px;
  font-weight: 600;
  line-height: 20px;
  white-space: nowrap;
}

.amane-badge--sm {
  padding: 0 6px;
  font-size: 11px;
  line-height: 18px;
}

.amane-badge--brand.amane-badge--light {
  background: var(--amane-brand-soft);
  color: var(--amane-brand-text);
}

.amane-badge--brand.amane-badge--filled {
  background: var(--amane-brand);
  color: #fff;
}

.amane-badge--brand.amane-badge--outline {
  border-color: var(--amane-brand);
  color: var(--amane-brand-text);
}

.amane-badge--success.amane-badge--light {
  background: var(--amane-success-soft);
  color: var(--amane-teal);
}

.amane-badge--success.amane-badge--filled {
  background: var(--amane-teal);
  color: #fff;
}

.amane-badge--warning.amane-badge--light {
  background: var(--amane-warning-soft);
  color: var(--amane-yellow);
}

.amane-badge--warning.amane-badge--filled {
  background: var(--amane-yellow);
  color: #1a1b1e;
}

.amane-badge--danger.amane-badge--light {
  background: var(--amane-error-soft);
  color: var(--amane-red);
}

.amane-badge--danger.amane-badge--filled {
  background: var(--amane-red);
  color: #fff;
}

.amane-badge--gray.amane-badge--light {
  background: var(--amane-surface-2);
  color: var(--amane-dimmed);
}

.amane-badge--gray.amane-badge--outline {
  border-color: var(--amane-line);
  color: var(--amane-dimmed);
}

.amane-badge--gray.amane-badge--filled {
  background: var(--amane-dimmed);
  color: var(--amane-body);
}

.amane-badge--outline.amane-badge--success {
  border-color: var(--amane-teal);
  color: var(--amane-teal);
}

.amane-badge--outline.amane-badge--warning {
  border-color: var(--amane-yellow);
  color: var(--amane-yellow);
}

.amane-badge--outline.amane-badge--danger {
  border-color: var(--amane-red);
  color: var(--amane-red);
}
</style>
