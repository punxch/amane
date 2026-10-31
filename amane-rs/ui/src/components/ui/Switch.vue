<script setup lang="ts">
const props = withDefaults(
  defineProps<{ modelValue: boolean; label: string; disabled?: boolean }>(),
  { disabled: false },
);

const emit = defineEmits<{ "update:modelValue": [value: boolean] }>();
</script>

<template>
  <button
    type="button"
    class="amane-switch"
    role="switch"
    :aria-checked="props.modelValue"
    :aria-label="props.label"
    :disabled="props.disabled"
    :data-checked="props.modelValue || undefined"
    @click="emit('update:modelValue', !props.modelValue)"
  >
    <span class="amane-switch__thumb" />
  </button>
</template>

<style scoped>
.amane-switch {
  position: relative;
  display: inline-flex;
  align-items: center;
  width: 38px;
  height: 20px;
  padding: 2px;
  border: 0;
  border-radius: 999px;
  background: var(--amane-surface-2);
  box-shadow: inset 0 0 0 1px var(--amane-line);
  cursor: pointer;
  transition: background-color 120ms ease;
}

.amane-switch[data-checked="true"] {
  background: var(--amane-brand);
  box-shadow: none;
}

.amane-switch:disabled {
  cursor: not-allowed;
  opacity: 0.5;
}

.amane-switch:focus-visible {
  outline: 2px solid var(--amane-brand-4);
  outline-offset: 2px;
}

.amane-switch__thumb {
  width: 14px;
  height: 14px;
  border-radius: 50%;
  background: #fff;
  transition: transform 120ms ease;
}

.amane-switch[data-checked="true"] .amane-switch__thumb {
  transform: translateX(18px);
}
</style>
