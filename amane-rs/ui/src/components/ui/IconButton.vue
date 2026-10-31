<script setup lang="ts">
import { computed } from "vue";

const props = withDefaults(
  defineProps<{
    variant?: "filled" | "default" | "subtle" | "light";
    size?: "sm" | "md";
    disabled?: boolean;
    label: string;
  }>(),
  { variant: "subtle", size: "md", disabled: false },
);

const classes = computed(() => [
  "amane-icon-button",
  `amane-icon-button--${props.variant}`,
  `amane-icon-button--${props.size}`,
]);
</script>

<template>
  <button :class="classes" :disabled="props.disabled" :aria-label="props.label" type="button">
    <slot />
  </button>
</template>

<style scoped>
.amane-icon-button {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 34px;
  height: 34px;
  border: 1px solid transparent;
  border-radius: var(--amane-radius);
  background: transparent;
  color: var(--amane-dimmed);
  cursor: pointer;
  transition:
    background-color 100ms ease,
    color 100ms ease,
    border-color 100ms ease;
}

.amane-icon-button--sm {
  width: 26px;
  height: 26px;
  border-radius: 6px;
}

.amane-icon-button:hover:not(:disabled) {
  background: var(--amane-surface-2);
  color: var(--amane-bright);
}

.amane-icon-button:focus-visible {
  outline: 2px solid var(--amane-brand-4);
  outline-offset: 1px;
}

.amane-icon-button:disabled {
  cursor: not-allowed;
  opacity: 0.45;
}

.amane-icon-button--default {
  border-color: var(--amane-line);
  background: var(--amane-surface);
  color: var(--amane-fg);
}

.amane-icon-button--light {
  background: var(--amane-brand-soft);
  color: var(--amane-brand-text);
}

.amane-icon-button--filled {
  background: var(--amane-brand);
  color: #fff;
}

.amane-icon-button--filled:hover:not(:disabled) {
  background: var(--amane-brand-hover);
  color: #fff;
}
</style>
