<script setup lang="ts">
defineProps<{ label: string }>();
</script>

<template>
  <!--
    悬浮说明. 不用 HTML `title`: 原生提示延迟出现、贴指针, 与原项目用 Tooltip 的观感不一致.
    disabled 的子元素不派发指针事件, 因此外层容器始终参与命中检测.
  -->
  <span class="amane-tooltip" :data-tip="label">
    <slot />
  </span>
</template>

<style scoped>
.amane-tooltip {
  position: relative;
  display: inline-flex;
}

.amane-tooltip::after {
  content: attr(data-tip);
  position: absolute;
  bottom: calc(100% + 8px);
  left: 50%;
  z-index: 70;
  padding: 4px 8px;
  border-radius: 4px;
  background: var(--amane-surface-2);
  box-shadow: 0 2px 8px rgb(0 0 0 / 28%);
  color: var(--amane-bright);
  font-size: 12px;
  line-height: 1.4;
  white-space: nowrap;
  pointer-events: none;
  opacity: 0;
  transform: translate(-50%, 2px);
  transition:
    opacity 120ms ease 150ms,
    transform 120ms ease 150ms;
}

.amane-tooltip:hover::after,
.amane-tooltip:focus-within::after {
  opacity: 1;
  transform: translate(-50%, 0);
}
</style>
