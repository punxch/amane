<script setup lang="ts">
import { computed } from "vue";

import Icon from "@/components/ui/Icon.vue";

const props = defineProps<{ totalPages: number; page: number }>();
const emit = defineEmits<{ change: [page: number] }>();

type PageItem = { kind: "page"; value: number } | { kind: "gap"; key: string };

/** 页码收合: 超过 7 页时只保留首末页与当前页前后一页, 中间折成省略号. */
const items = computed<PageItem[]>(() => {
  const total = props.totalPages;
  if (total <= 7) {
    return Array.from({ length: total }, (_, index) => ({ kind: "page" as const, value: index + 1 }));
  }

  const anchors = [1, total, props.page - 1, props.page, props.page + 1]
    .filter((value) => value >= 1 && value <= total)
    .sort((left, right) => left - right);

  const result: PageItem[] = [];
  let previous = 0;
  for (const value of anchors) {
    if (value - previous > 1) {
      result.push({ kind: "gap", key: `gap-${previous}` });
    }
    result.push({ kind: "page", value });
    previous = value;
  }
  return result;
});

function go(page: number): void {
  if (page < 1 || page > props.totalPages || page === props.page) {
    return;
  }
  emit("change", page);
}
</script>

<template>
  <nav v-if="props.totalPages > 1" class="amane-pagination" aria-label="分页">
    <button
      type="button"
      class="amane-pagination__arrow"
      :disabled="props.page <= 1"
      aria-label="上一页"
      @click="go(props.page - 1)"
    >
      <Icon name="chevron-left" :size="16" />
    </button>

    <template v-for="item in items" :key="item.kind === 'page' ? item.value : item.key">
      <span v-if="item.kind === 'gap'" class="amane-pagination__gap">…</span>
      <button
        v-else
        type="button"
        class="amane-pagination__page"
        :data-active="item.value === props.page || undefined"
        :aria-current="item.value === props.page ? 'page' : undefined"
        @click="go(item.value)"
      >
        {{ item.value }}
      </button>
    </template>

    <button
      type="button"
      class="amane-pagination__arrow"
      :disabled="props.page >= props.totalPages"
      aria-label="下一页"
      @click="go(props.page + 1)"
    >
      <Icon name="chevron-right" :size="16" />
    </button>
  </nav>
</template>

<style scoped>
.amane-pagination {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 4px;
}

.amane-pagination__page,
.amane-pagination__arrow {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-width: 30px;
  height: 30px;
  padding: 0 8px;
  border: 1px solid transparent;
  border-radius: 6px;
  background: transparent;
  color: var(--amane-fg);
  font-size: 13px;
  font-variant-numeric: tabular-nums;
  cursor: pointer;
  transition:
    background-color 100ms ease,
    color 100ms ease;
}

.amane-pagination__page:hover,
.amane-pagination__arrow:hover:not(:disabled) {
  background: var(--amane-surface-2);
}

.amane-pagination__page[data-active="true"] {
  background: var(--amane-brand);
  color: #fff;
  font-weight: 600;
}

.amane-pagination__arrow:disabled {
  color: var(--amane-subtle);
  cursor: not-allowed;
}

.amane-pagination__gap {
  min-width: 24px;
  color: var(--amane-dimmed);
  text-align: center;
}
</style>
