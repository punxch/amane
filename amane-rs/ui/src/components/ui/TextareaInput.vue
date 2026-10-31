<script setup lang="ts">
const props = withDefaults(
  defineProps<{
    modelValue: string;
    placeholder?: string;
    rows?: number;
    mono?: boolean;
    disabled?: boolean;
    ariaLabel?: string;
  }>(),
  { placeholder: "", rows: 3, mono: false, disabled: false, ariaLabel: "" },
);

const emit = defineEmits<{ "update:modelValue": [value: string] }>();

/** 取值经事件目标收窄, 而不是把 DOM 事件断言成 TextArea 元素. */
function onInput(event: Event): void {
  const target = event.target;
  if (target instanceof HTMLTextAreaElement) {
    emit("update:modelValue", target.value);
  }
}
</script>

<template>
  <textarea
    :value="props.modelValue"
    :rows="props.rows"
    :placeholder="props.placeholder"
    :disabled="props.disabled"
    :aria-label="props.ariaLabel === '' ? undefined : props.ariaLabel"
    :class="['amane-textarea', props.mono ? 'amane-textarea--mono' : '']"
    @input="onInput"
  />
</template>

<style scoped>
.amane-textarea {
  width: 100%;
  padding: 8px 12px;
  border: 1px solid var(--amane-line);
  border-radius: var(--amane-radius);
  background: var(--amane-body);
  color: var(--amane-fg);
  font: inherit;
  font-size: 14px;
  line-height: 1.5;
  resize: vertical;
  outline: none;
  transition:
    border-color 100ms ease,
    box-shadow 100ms ease;
}

.amane-textarea::placeholder {
  color: var(--amane-subtle);
}

.amane-textarea:hover:not(:disabled) {
  border-color: var(--amane-dimmed);
}

.amane-textarea:focus {
  border-color: var(--amane-brand);
  box-shadow: 0 0 0 2px var(--amane-brand-soft);
}

.amane-textarea--mono {
  font-family: var(--font-mono);
  font-size: 13px;
}
</style>
