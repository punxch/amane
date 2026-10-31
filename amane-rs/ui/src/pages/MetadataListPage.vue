<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { RouterLink, useRoute, useRouter, type LocationQueryValue } from "vue-router";

import { api, describeError, type FilterOptions, type MetadataDto } from "@/api/client";
import EmptyState from "@/components/common/EmptyState.vue";
import PageShell from "@/components/common/PageShell.vue";
import Pagination from "@/components/common/Pagination.vue";
import HighlightedText from "@/components/HighlightedText.vue";
import Alert from "@/components/ui/Alert.vue";
import Button from "@/components/ui/Button.vue";
import Icon from "@/components/ui/Icon.vue";
import Paper from "@/components/ui/Paper.vue";
import SelectInput, { type SelectOption } from "@/components/ui/SelectInput.vue";
import { formatDate, formatTime, pageCount } from "@/lib/format";
import { PAGE_SIZE_OPTIONS, setPageSize, ui } from "@/stores/ui";

type ViewMode = "grid" | "table";

const route = useRoute();
const router = useRouter();

const items = ref<MetadataDto[]>([]);
const total = ref(0);
const loading = ref(false);
const failure = ref<string | null>(null);
const filters = ref<FilterOptions | null>(null);
const filtersFailed = ref(false);

/** URL 是列表态的唯一事实来源: 筛选、页码与视图都写进地址栏, 刷新与分享后一致. */
function readQuery(value: LocationQueryValue | LocationQueryValue[] | undefined): string {
  return typeof value === "string" ? value : "";
}

const q = computed(() => readQuery(route.query.q));
const contentType = computed(() => readQuery(route.query.type));
const source = computed(() => readQuery(route.query.source));
const genre = computed(() => readQuery(route.query.genre));
const view = computed<ViewMode>(() => (readQuery(route.query.view) === "table" ? "table" : "grid"));
const page = computed(() => {
  const parsed = Number.parseInt(readQuery(route.query.page), 10);
  return Number.isNaN(parsed) || parsed < 1 ? 1 : parsed;
});
const pages = computed(() => pageCount(total.value, ui.pageSize));

const keywordDraft = ref(q.value);
let searchTimer: ReturnType<typeof setTimeout> | undefined;

const hasFilter = computed(
  () =>
    q.value.trim() !== "" ||
    contentType.value !== "" ||
    source.value !== "" ||
    genre.value !== "",
);

const typeOptions = computed<SelectOption[]>(() =>
  (filters.value?.content_types ?? []).map((option) => ({
    value: option.value,
    label: option.label,
  })),
);

const sourceOptions = computed<SelectOption[]>(() =>
  (filters.value?.sources ?? []).map((option) => ({ value: option.id, label: option.label })),
);

const genreOptions = computed<SelectOption[]>(() =>
  (filters.value?.genres ?? []).map((name) => ({ value: name, label: name })),
);

const pageSizeOptions: SelectOption[] = PAGE_SIZE_OPTIONS.map((size) => ({
  value: String(size),
  label: `${size} / 页`,
}));

const subtitle = computed(() => {
  const count = loading.value ? "读取中" : `${total.value} 条`;
  return hasFilter.value ? `${count} · 已筛选` : count;
});

function currentQuery(): Record<string, string> {
  const query: Record<string, string> = {};
  for (const [key, value] of Object.entries(route.query)) {
    if (typeof value === "string" && value !== "") {
      query[key] = value;
    }
  }
  return query;
}

/** 写入 URL; `null` 表示删除该参数 (例如筛选变化后回到第一页). */
function updateQuery(patch: Record<string, string | null>): void {
  const next = currentQuery();
  for (const [key, value] of Object.entries(patch)) {
    if (value === null || value === "") {
      delete next[key];
    } else {
      next[key] = value;
    }
  }
  void router.replace({ query: next });
}

let requestSeq = 0;

async function load(): Promise<void> {
  // 防抖只决定何时发请求; 慢查询下旧响应仍可能后到, 用它丢弃过期结果.
  const seq = ++requestSeq;
  loading.value = true;
  failure.value = null;
  try {
    const result = await api.list({
      keyword: q.value,
      contentType: contentType.value,
      source: source.value,
      genre: genre.value,
      limit: ui.pageSize,
      offset: (page.value - 1) * ui.pageSize,
    });
    if (seq !== requestSeq) {
      return;
    }
    // 筛选后总页数可能变少: 停在越界页时列表为空, 回到最后一页.
    if (result.items.length === 0 && result.total > 0 && page.value > 1) {
      updateQuery({ page: String(pageCount(result.total, ui.pageSize)) });
      total.value = result.total;
      return;
    }
    items.value = result.items;
    total.value = result.total;
  } catch (error) {
    if (seq !== requestSeq) {
      return;
    }
    // 保留总数: 否则页码会显示成「第 2 / 1 页」.
    items.value = [];
    failure.value = describeError(error);
  } finally {
    if (seq === requestSeq) {
      loading.value = false;
    }
  }
}

function typeLabel(value: string): string {
  const option = filters.value?.content_types.find((entry) => entry.value === value);
  return option?.label ?? value;
}

function clearFilters(): void {
  keywordDraft.value = "";
  updateQuery({ q: null, type: null, source: null, genre: null, page: null });
}

/** 每页条数只接受可选列表里的取值; 其余输入原样忽略, 不写进偏好. */
function changePageSize(value: string): void {
  const size = PAGE_SIZE_OPTIONS.find((option) => String(option) === value);
  if (size === undefined) {
    return;
  }
  setPageSize(size);
  updateQuery({ page: null });
}

function applyKeyword(): void {
  if (searchTimer !== undefined) {
    clearTimeout(searchTimer);
    searchTimer = undefined;
  }
  if (keywordDraft.value.trim() === q.value) {
    return;
  }
  updateQuery({ q: keywordDraft.value.trim(), page: null });
}

// 地址栏被顶栏搜索改写时, 同步回输入框; 反之输入停顿 250ms 后写回地址栏.
watch(q, (value) => {
  if (value !== keywordDraft.value) {
    keywordDraft.value = value;
  }
});

watch(keywordDraft, (value) => {
  if (value === q.value) {
    return;
  }
  if (searchTimer !== undefined) {
    clearTimeout(searchTimer);
  }
  searchTimer = setTimeout(() => {
    updateQuery({ q: value.trim(), page: null });
  }, 250);
});

watch([q, contentType, source, genre, page, () => ui.pageSize], () => void load());

onBeforeUnmount(() => {
  if (searchTimer !== undefined) {
    clearTimeout(searchTimer);
  }
});

onMounted(async () => {
  // 可选项只是下拉框内容, 取不到也不阻塞列表本身.
  void api
    .filters()
    .then((value) => {
      filters.value = value;
      filtersFailed.value = false;
    })
    .catch(() => {
      filters.value = null;
      filtersFailed.value = true;
    });
  await load();
});
</script>

<template>
  <PageShell title="片库" :subtitle="subtitle" fill>
    <template #actions>
      <Button variant="default" :disabled="loading" @click="load">刷新</Button>
    </template>

    <template #toolbar>
      <input
        v-model="keywordDraft"
        class="toolbar-search"
        type="search"
        placeholder="搜索番号或标题"
        aria-label="搜索番号或标题"
        @keydown.enter.prevent="applyKeyword"
      />
      <SelectInput
        :model-value="contentType"
        :options="typeOptions"
        placeholder="全部类型"
        aria-label="类型"
        @update:model-value="(value) => updateQuery({ type: value, page: null })"
      />
      <SelectInput
        :model-value="source"
        :options="sourceOptions"
        placeholder="全部来源"
        aria-label="来源"
        @update:model-value="(value) => updateQuery({ source: value, page: null })"
      />
      <SelectInput
        :model-value="genre"
        :options="genreOptions"
        placeholder="全部题材"
        aria-label="题材"
        @update:model-value="(value) => updateQuery({ genre: value, page: null })"
      />
      <Button v-if="hasFilter" variant="subtle" size="sm" @click="clearFilters">清除筛选</Button>

      <span class="toolbar-spacer" />

      <div class="segmented" role="group" aria-label="视图">
        <button
          type="button"
          :data-active="view === 'grid' || undefined"
          @click="updateQuery({ view: null })"
        >
          海报
        </button>
        <button
          type="button"
          :data-active="view === 'table' || undefined"
          @click="updateQuery({ view: 'table' })"
        >
          表格
        </button>
      </div>

      <SelectInput
        :model-value="String(ui.pageSize)"
        :options="pageSizeOptions"
        size="sm"
        aria-label="每页条数"
        class="page-size"
        @update:model-value="changePageSize"
      />
    </template>

    <Alert v-if="filtersFailed" tone="warning" class="filters-warning">
      筛选项未加载，只能按关键字检索。
    </Alert>

    <div class="list">
      <div class="list__body">
        <Alert v-if="failure !== null" tone="danger">
          {{ failure }}
          <template #action>
            <Button variant="subtle" size="sm" @click="load">重试</Button>
          </template>
        </Alert>

        <EmptyState
          v-else-if="loading && items.length === 0"
          title="读取中…"
          description="正在向后端查询片库。"
        />

        <EmptyState
          v-else-if="items.length === 0 && hasFilter"
          icon="search"
          title="没有符合条件的影片"
          description="换一个关键字，或清除筛选后重试。"
        >
          <Button variant="default" size="sm" @click="clearFilters">清除筛选</Button>
        </EmptyState>

        <EmptyState
          v-else-if="items.length === 0"
          icon="movie"
          title="尚无影片"
          description="在任务页提交一次刮削，写入的影片会出现在这里。"
        >
          <RouterLink to="/tasks">
            <Button size="sm">去提交刮削</Button>
          </RouterLink>
        </EmptyState>

        <div v-else-if="view === 'grid'" class="poster-grid">
          <RouterLink
            v-for="item in items"
            :key="item.id"
            class="poster-card"
            :to="{ name: 'metadata-detail', params: { number: item.number } }"
          >
            <div class="poster-card__frame">
              <img
                v-if="item.poster_local_url"
                :src="item.poster_local_url"
                :alt="item.title ?? item.number"
                loading="lazy"
              />
              <div v-else class="poster-card__placeholder">
                <Icon name="movie" :size="26" :stroke="1.3" />
              </div>
            </div>
            <div class="poster-card__body">
              <p class="poster-card__number">
                <HighlightedText :text="item.number" :keyword="q" />
              </p>
              <p class="poster-card__title">
                <HighlightedText v-if="item.title" :text="item.title" :keyword="q" />
                <template v-else>—</template>
              </p>
              <p class="poster-card__meta">
                {{ item.maker ?? "—" }} · {{ formatDate(item.release_date) }}
              </p>
            </div>
          </RouterLink>
        </div>

        <Paper v-else padding="none">
          <table class="amane-table">
            <thead>
              <tr>
                <th style="width: 140px">番号</th>
                <th>标题</th>
                <th style="width: 110px">类型</th>
                <th style="width: 140px">制作商</th>
                <th style="width: 120px">发行日期</th>
                <th style="width: 170px">更新时间</th>
              </tr>
            </thead>
            <tbody>
              <tr
                v-for="item in items"
                :key="item.id"
                data-clickable="true"
                @click="
                  router.push({ name: 'metadata-detail', params: { number: item.number } })
                "
              >
                <td style="font-family: var(--font-mono)">
                  <HighlightedText :text="item.number" :keyword="q" />
                </td>
                <td class="cell-title">
                  <HighlightedText v-if="item.title" :text="item.title" :keyword="q" />
                  <template v-else>—</template>
                </td>
                <td class="cell-dimmed">{{ typeLabel(item.content_type) }}</td>
                <td class="cell-dimmed">{{ item.maker ?? "—" }}</td>
                <td class="cell-dimmed mono">{{ formatDate(item.release_date) }}</td>
                <td class="cell-dimmed mono">{{ formatTime(item.updated_at) }}</td>
              </tr>
            </tbody>
          </table>
        </Paper>
      </div>

      <div v-if="pages > 1" class="list__footer">
        <Pagination
          :total-pages="pages"
          :page="page"
          @change="(value) => updateQuery({ page: String(value) })"
        />
      </div>
    </div>
  </PageShell>
</template>

<style scoped>
.toolbar-search {
  width: 240px;
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

.toolbar-search::placeholder {
  color: var(--amane-subtle);
}

.toolbar-search:focus {
  border-color: var(--amane-brand);
  box-shadow: 0 0 0 2px var(--amane-brand-soft);
}

.toolbar-spacer {
  flex: 1;
  min-width: 8px;
}

.segmented {
  display: inline-flex;
  padding: 2px;
  border: 1px solid var(--amane-line);
  border-radius: var(--amane-radius);
  background: var(--amane-surface);
}

.segmented button {
  height: 28px;
  padding: 0 12px;
  border: 0;
  border-radius: 6px;
  background: transparent;
  color: var(--amane-dimmed);
  font-size: 13px;
  cursor: pointer;
}

.segmented button[data-active="true"] {
  background: var(--amane-surface-2);
  color: var(--amane-bright);
  font-weight: 600;
}

.filters-warning {
  margin-bottom: 12px;
}

.list {
  display: flex;
  flex: 1;
  flex-direction: column;
  gap: 12px;
  min-height: 0;
}

.list__body {
  flex: 1;
  min-height: 0;
  overflow: auto;
}

.list__footer {
  flex-shrink: 0;
  padding-top: 4px;
}

.poster-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(148px, 1fr));
  gap: 16px 12px;
  padding-bottom: 8px;
}

.poster-card {
  display: flex;
  flex-direction: column;
  min-width: 0;
  overflow: hidden;
  border: 1px solid var(--amane-line);
  border-radius: var(--amane-radius);
  background: var(--amane-surface);
  transition: border-color 120ms ease;
}

.poster-card:hover {
  border-color: var(--amane-brand);
}

.poster-card__frame {
  position: relative;
  overflow: hidden;
  aspect-ratio: 2 / 3;
  background: var(--amane-surface-2);
}

.poster-card__frame img {
  width: 100%;
  height: 100%;
  object-fit: cover;
  display: block;
}

.poster-card__placeholder {
  display: flex;
  align-items: center;
  justify-content: center;
  height: 100%;
  color: var(--amane-subtle);
}

.poster-card__body {
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 8px 10px 10px;
}

.poster-card__number {
  margin: 0;
  color: var(--amane-bright);
  font-family: var(--font-mono);
  font-size: 13px;
  font-weight: 600;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.poster-card__title {
  margin: 0;
  display: -webkit-box;
  overflow: hidden;
  color: var(--amane-dimmed);
  font-size: 12px;
  line-height: 1.45;
  -webkit-box-orient: vertical;
  -webkit-line-clamp: 2;
}

.poster-card__meta {
  margin: 0;
  overflow: hidden;
  color: var(--amane-subtle);
  font-size: 11px;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.cell-title {
  color: var(--amane-fg);
}

.cell-dimmed {
  color: var(--amane-dimmed);
}

.mono {
  font-family: var(--font-mono);
  font-size: 12px;
}

.page-size {
  min-width: 110px;
}
</style>
