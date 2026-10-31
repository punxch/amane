<script setup lang="ts">
const props = withDefaults(
  defineProps<{
    title: string;
    /** 标题行下方的补充说明 (计数、范围等). */
    subtitle?: string;
    /** 撑满主内容区, 把剩余高度交给内容: 列表页用来把分页钉在视口底. */
    fill?: boolean;
  }>(),
  { subtitle: "", fill: false },
);
</script>

<template>
  <div class="amane-page" :class="props.fill ? 'amane-page--fill' : ''">
    <header class="amane-page__head">
      <div class="amane-page__title-row">
        <div class="amane-page__title">
          <h1>{{ props.title }}</h1>
          <p v-if="props.subtitle !== ''" class="amane-page__subtitle">{{ props.subtitle }}</p>
        </div>
        <div v-if="$slots.actions" class="amane-page__actions">
          <slot name="actions" />
        </div>
      </div>
      <div v-if="$slots.toolbar" class="amane-page__toolbar">
        <slot name="toolbar" />
      </div>
    </header>

    <div class="amane-page__body">
      <slot />
    </div>
  </div>
</template>

<style scoped>
.amane-page {
  display: flex;
  flex-direction: column;
  gap: 16px;
  min-width: 0;
}

.amane-page--fill {
  height: 100%;
  min-height: 0;
}

.amane-page__head {
  display: flex;
  flex-direction: column;
  gap: 14px;
  flex-shrink: 0;
  padding-bottom: 16px;
  border-bottom: 1px solid var(--amane-line);
}

.amane-page__title-row {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 16px;
  flex-wrap: wrap;
}

.amane-page__title {
  min-width: 0;
}

.amane-page__title h1 {
  font-size: 22px;
  letter-spacing: -0.01em;
}

.amane-page__subtitle {
  margin: 4px 0 0;
  color: var(--amane-dimmed);
  font-size: 13px;
}

.amane-page__actions {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}

.amane-page__toolbar {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
  min-width: 0;
}

.amane-page__body {
  min-width: 0;
}

.amane-page--fill .amane-page__body {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
}
</style>
