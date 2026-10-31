<script setup lang="ts">
import { computed } from "vue";

const props = withDefaults(
  defineProps<{
    variant?: "filled" | "default" | "subtle" | "light" | "danger";
    size?: "sm" | "md";
    type?: "button" | "submit";
    disabled?: boolean;
    loading?: boolean;
    block?: boolean;
  }>(),
  {
    variant: "filled",
    size: "md",
    type: "button",
    disabled: false,
    loading: false,
    block: false,
  },
);

const classes = computed(() => [
  "amane-button",
  `amane-button--${props.variant}`,
  `amane-button--${props.size}`,
  props.block ? "amane-button--block" : "",
]);
</script>

<template>
  <button
    :type="props.type"
    :class="classes"
    :disabled="props.disabled || props.loading"
    :data-loading="props.loading || undefined"
  >
    <span v-if="props.loading" class="amane-button__spinner" aria-hidden="true" />
    <slot />
  </button>
</template>

<style scoped>
.amane-button {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 8px;
  height: 36px;
  padding: 0 18px;
  border: 1px solid transparent;
  border-radius: var(--amane-radius);
  font-size: 14px;
  font-weight: 600;
  line-height: 1;
  white-space: nowrap;
  cursor: pointer;
  transition:
    background-color 100ms ease,
    border-color 100ms ease,
    color 100ms ease;
}

.amane-button:disabled {
  cursor: not-allowed;
  opacity: 0.55;
}

.amane-button:focus-visible {
  outline: 2px solid var(--amane-brand-4);
  outline-offset: 1px;
}

.amane-button--sm {
  height: 30px;
  padding: 0 12px;
  font-size: 13px;
}

.amane-button--block {
  width: 100%;
}

.amane-button--filled {
  background: var(--amane-brand);
  color: #fff;
}

.amane-button--filled:hover:not(:disabled) {
  background: var(--amane-brand-hover);
}

.amane-button--default {
  background: var(--amane-surface);
  border-color: var(--amane-line);
  color: var(--amane-fg);
}

.amane-button--default:hover:not(:disabled) {
  background: var(--amane-surface-2);
}

.amane-button--subtle {
  background: transparent;
  color: var(--amane-fg);
}

.amane-button--subtle:hover:not(:disabled) {
  background: var(--amane-surface-2);
}

.amane-button--light {
  background: var(--amane-brand-soft);
  color: var(--amane-brand-text);
}

.amane-button--light:hover:not(:disabled) {
  background: color-mix(in srgb, var(--amane-brand) 24%, transparent);
}

.amane-button--danger {
  background: var(--amane-red);
  color: #fff;
}

.amane-button--danger:hover:not(:disabled) {
  background: #e03131;
}

.amane-button__spinner {
  width: 12px;
  height: 12px;
  border: 2px solid currentColor;
  border-right-color: transparent;
  border-radius: 50%;
  animation: amane-spin 700ms linear infinite;
}

@keyframes amane-spin {
  to {
    transform: rotate(360deg);
  }
}
</style>
