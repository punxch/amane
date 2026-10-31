<script setup lang="ts">
import { onBeforeUnmount, onMounted, watch } from "vue";

import IconButton from "@/components/ui/IconButton.vue";
import Icon from "@/components/ui/Icon.vue";

const props = withDefaults(
  defineProps<{
    opened: boolean;
    title: string;
    /** 对话框宽度上限, 单位 px. */
    width?: number;
  }>(),
  { width: 480 },
);

const emit = defineEmits<{ close: [] }>();

/** Esc 关闭; 只在打开时监听, 多个对话框叠加时最上层先响应. */
function onKeydown(event: KeyboardEvent): void {
  if (event.key === "Escape" && props.opened) {
    emit("close");
  }
}

watch(
  () => props.opened,
  (opened) => {
    document.body.style.overflow = opened ? "hidden" : "";
  },
);

onMounted(() => window.addEventListener("keydown", onKeydown));

onBeforeUnmount(() => {
  window.removeEventListener("keydown", onKeydown);
  document.body.style.overflow = "";
});
</script>

<template>
  <Teleport to="body">
    <div v-if="props.opened" class="amane-modal">
      <div class="amane-modal__overlay" @click="emit('close')" />
      <div
        class="amane-modal__dialog"
        role="dialog"
        aria-modal="true"
        :aria-label="props.title"
        :style="{ maxWidth: `${props.width}px` }"
      >
        <header class="amane-modal__header">
          <h2 class="amane-modal__title">{{ props.title }}</h2>
          <IconButton label="关闭" @click="emit('close')">
            <Icon name="close" :size="16" />
          </IconButton>
        </header>
        <div class="amane-modal__body">
          <slot />
        </div>
        <footer v-if="$slots.footer" class="amane-modal__footer">
          <slot name="footer" />
        </footer>
      </div>
    </div>
  </Teleport>
</template>

<style scoped>
.amane-modal {
  position: fixed;
  inset: 0;
  z-index: 60;
  display: flex;
  align-items: flex-start;
  justify-content: center;
  padding: 10vh 16px 16px;
  overflow-y: auto;
}

.amane-modal__overlay {
  position: fixed;
  inset: 0;
  background: var(--amane-overlay);
}

.amane-modal__dialog {
  position: relative;
  width: 100%;
  border: 1px solid var(--amane-line);
  border-radius: calc(var(--amane-radius) + 2px);
  background: var(--amane-surface);
  box-shadow: var(--amane-shadow);
}

.amane-modal__header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 14px 18px;
  border-bottom: 1px solid var(--amane-line);
}

.amane-modal__title {
  font-size: 16px;
}

.amane-modal__body {
  display: flex;
  flex-direction: column;
  gap: 14px;
  padding: 18px;
}

.amane-modal__footer {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  padding: 12px 18px;
  border-top: 1px solid var(--amane-line);
}
</style>
