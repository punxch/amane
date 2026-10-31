<script setup lang="ts">
const props = withDefaults(
  defineProps<{
    modelValue: string;
    placeholder?: string;
    type?: "text" | "number" | "password" | "url";
    size?: "sm" | "md";
    mono?: boolean;
    disabled?: boolean;
    invalid?: boolean;
    ariaLabel?: string;
  }>(),
  {
    placeholder: "",
    type: "text",
    size: "md",
    mono: false,
    disabled: false,
    invalid: false,
    ariaLabel: "",
  },
);

const emit = defineEmits<{ "update:modelValue": [value: string] }>();

/** 取值经事件目标收窄, 而不是把 DOM 事件断言成 Input 元素. */
function onInput(event: Event): void {
  const target = event.target;
  if (target instanceof HTMLInputElement) {
    emit("update:modelValue", target.value);
  }
}
</script>

<template>
  <input
    :value="props.modelValue"
    :type="props.type"
    :placeholder="props.placeholder"
    :disabled="props.disabled"
    :aria-label="props.ariaLabel === '' ? undefined : props.ariaLabel"
    :data-invalid="props.invalid || undefined"
    :class="[
      'amane-input',
      props.size === 'sm' ? 'amane-input--sm' : '',
      props.mono ? 'amane-input--mono' : '',
    ]"
    @input="onInput"
  />
</template>

<style scoped>
.amane-input {
  width: 100%;
  height: 36px;
  padding: 0 12px;
  border: 1px solid var(--amane-line);
  border-radius: var(--amane-radius);
  background: var(--amane-body);
  color: var(--amane-fg);
  font: inherit;
  font-size: 14px;
  outline: none;
  transition:
    border-color 100ms ease,
    box-shadow 100ms ease;
}

.amane-input::placeholder {
  color: var(--amane-subtle);
}

.amane-input:hover:not(:disabled) {
  border-color: var(--amane-dimmed);
}

.amane-input:focus {
  border-color: var(--amane-brand);
  box-shadow: 0 0 0 2px var(--amane-brand-soft);
}

.amane-input:disabled {
  background: var(--amane-surface);
  color: var(--amane-dimmed);
  cursor: not-allowed;
}

.amane-input[data-invalid="true"] {
  border-color: var(--amane-red);
}

.amane-input--sm {
  height: 30px;
  padding: 0 10px;
  font-size: 13px;
}

.amane-input--mono {
  font-family: var(--font-mono);
}
</style>
