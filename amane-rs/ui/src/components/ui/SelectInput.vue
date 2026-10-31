<script setup lang="ts">
export interface SelectOption {
  value: string;
  label: string;
}

const props = withDefaults(
  defineProps<{
    modelValue: string;
    options: readonly SelectOption[];
    placeholder?: string;
    size?: "sm" | "md";
    disabled?: boolean;
    ariaLabel?: string;
  }>(),
  { placeholder: "", size: "md", disabled: false, ariaLabel: "" },
);

const emit = defineEmits<{ "update:modelValue": [value: string] }>();

function onChange(event: Event): void {
  const target = event.target;
  if (target instanceof HTMLSelectElement) {
    emit("update:modelValue", target.value);
  }
}
</script>

<template>
  <div class="amane-select" :class="props.size === 'sm' ? 'amane-select--sm' : ''">
    <select
      :value="props.modelValue"
      :disabled="props.disabled"
      :aria-label="props.ariaLabel === '' ? undefined : props.ariaLabel"
      @change="onChange"
    >
      <option value="">{{ props.placeholder }}</option>
      <option v-for="option in props.options" :key="option.value" :value="option.value">
        {{ option.label }}
      </option>
    </select>
    <svg class="amane-select__chevron" viewBox="0 0 24 24" aria-hidden="true">
      <path d="M6 9l6 6l6 -6" fill="none" stroke="currentColor" stroke-width="1.6" />
    </svg>
  </div>
</template>

<style scoped>
.amane-select {
  position: relative;
  display: inline-flex;
  min-width: 120px;
}

.amane-select select {
  width: 100%;
  height: 36px;
  padding: 0 30px 0 12px;
  border: 1px solid var(--amane-line);
  border-radius: var(--amane-radius);
  background: var(--amane-body);
  color: var(--amane-fg);
  font: inherit;
  font-size: 14px;
  line-height: 1;
  appearance: none;
  cursor: pointer;
  outline: none;
  transition:
    border-color 100ms ease,
    box-shadow 100ms ease;
}

.amane-select select:hover:not(:disabled) {
  border-color: var(--amane-dimmed);
}

.amane-select select:focus {
  border-color: var(--amane-brand);
  box-shadow: 0 0 0 2px var(--amane-brand-soft);
}

.amane-select select:disabled {
  color: var(--amane-dimmed);
  cursor: not-allowed;
}

.amane-select--sm select {
  height: 30px;
  padding-left: 10px;
  font-size: 13px;
}

.amane-select__chevron {
  position: absolute;
  top: 50%;
  right: 8px;
  width: 16px;
  height: 16px;
  transform: translateY(-50%);
  color: var(--amane-dimmed);
  pointer-events: none;
}
</style>
