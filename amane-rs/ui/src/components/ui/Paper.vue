<script setup lang="ts">
withDefaults(
  defineProps<{
    /** 内边距; `none` 用于让表格或列表自己贴边. */
    padding?: "none" | "sm" | "md" | "lg";
    /** 面板底色; 默认抬一层, 与页面背景区分. */
    flat?: boolean;
  }>(),
  { padding: "md", flat: false },
);
</script>

<template>
  <section
    class="amane-paper"
    :class="[`amane-paper--${padding}`, flat ? 'amane-paper--flat' : '']"
  >
    <slot />
  </section>
</template>

<style scoped>
.amane-paper {
  border: 1px solid var(--amane-line);
  border-radius: var(--amane-radius);
  background: var(--amane-surface);
  min-width: 0;
}

.amane-paper--flat {
  background: transparent;
}

.amane-paper--none {
  padding: 0;
}

.amane-paper--sm {
  padding: 12px;
}

.amane-paper--md {
  padding: 16px;
}

.amane-paper--lg {
  padding: 24px;
}
</style>
