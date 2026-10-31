<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";

import { api, describeError, type LibraryDto, type LibraryFilePage } from "@/api/client";
import EmptyState from "@/components/common/EmptyState.vue";
import Pagination from "@/components/common/Pagination.vue";
import Alert from "@/components/ui/Alert.vue";
import Badge from "@/components/ui/Badge.vue";
import Button from "@/components/ui/Button.vue";
import Paper from "@/components/ui/Paper.vue";
import { pageCount } from "@/lib/format";
import { SCAN_KINDS, ruleLabel } from "@/lib/library/kinds";

const props = defineProps<{ library: LibraryDto }>();
const emit = defineEmits<{ notice: [tone: "error" | "ok", text: string] }>();

const PAGE_SIZE = 100;

const kind = ref<string>("");
const page = ref(1);
const files = ref<LibraryFilePage | null>(null);
const loading = ref(false);
const failure = ref<string | null>(null);

const pages = computed(() => pageCount(files.value?.total ?? 0, PAGE_SIZE));

const kindOptions = computed(() => [
  { value: "", label: "全部", count: null as number | null },
  ...SCAN_KINDS.map((entry) => ({
    value: entry.value,
    label: entry.label,
    count: props.library.counts[entry.value] ?? 0,
  })),
]);

async function load(): Promise<void> {
  loading.value = true;
  failure.value = null;
  try {
    files.value = await api.libraryFiles(
      props.library.id,
      kind.value === "" ? null : kind.value,
      PAGE_SIZE,
      (page.value - 1) * PAGE_SIZE,
    );
  } catch (error) {
    files.value = null;
    failure.value = describeError(error);
  } finally {
    loading.value = false;
  }
}

function selectKind(value: string): void {
  if (value === kind.value) {
    return;
  }
  kind.value = value;
  page.value = 1;
  void load();
}

watch(page, () => void load());
onMounted(load);
</script>

<template>
  <div class="files">
    <div class="files__filters">
      <button
        v-for="option in kindOptions"
        :key="option.value"
        type="button"
        class="kind-chip"
        :data-active="kind === option.value || undefined"
        @click="selectKind(option.value)"
      >
        {{ option.label }}
        <span v-if="option.count !== null" class="kind-chip__count">{{ option.count }}</span>
      </button>
      <span class="files__spacer" />
      <Button variant="default" size="sm" :disabled="loading" @click="load">刷新</Button>
    </div>

    <Alert v-if="failure !== null" tone="danger">
      {{ failure }}
      <template #action>
        <Button variant="subtle" size="sm" @click="load">重试</Button>
      </template>
    </Alert>

    <EmptyState
      v-else-if="!loading && (files?.items.length ?? 0) === 0"
      icon="folder"
      title="该类别下没有文件"
      description="扫描后归类结果会出现在这里。"
    />

    <Paper v-else padding="none">
      <table class="amane-table">
        <thead>
          <tr>
            <th>相对路径</th>
            <th style="width: 160px">命中规则</th>
            <th style="width: 260px">说明</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="file in files?.items ?? []" :key="file.path">
            <td class="path">{{ file.path }}</td>
            <td>
              <Badge v-if="file.rule" tone="danger" size="sm">{{ ruleLabel(file.rule) }}</Badge>
              <span v-else class="dimmed">—</span>
            </td>
            <td class="dimmed small">{{ file.detail ?? "—" }}</td>
          </tr>
        </tbody>
      </table>
    </Paper>

    <div v-if="pages > 1" class="files__footer">
      <Pagination :total-pages="pages" :page="page" @change="(value) => (page = value)" />
    </div>
  </div>
</template>

<style scoped>
.files {
  display: flex;
  flex-direction: column;
  gap: 12px;
  min-width: 0;
}

.files__filters {
  display: flex;
  align-items: center;
  gap: 6px;
  flex-wrap: wrap;
}

.files__spacer {
  flex: 1;
}

.kind-chip {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  height: 30px;
  padding: 0 10px;
  border: 1px solid var(--amane-line);
  border-radius: 999px;
  background: transparent;
  color: var(--amane-dimmed);
  font-size: 13px;
  cursor: pointer;
}

.kind-chip:hover {
  color: var(--amane-bright);
}

.kind-chip[data-active="true"] {
  border-color: var(--amane-brand);
  background: var(--amane-brand-soft);
  color: var(--amane-brand-text);
}

.kind-chip__count {
  font-family: var(--font-mono);
  font-size: 11px;
  opacity: 0.8;
}

.path {
  color: var(--amane-fg);
  font-family: var(--font-mono);
  font-size: 12px;
  overflow-wrap: anywhere;
}

.dimmed {
  color: var(--amane-dimmed);
}

.small {
  font-size: 12px;
}

.files__footer {
  padding-top: 4px;
}
</style>
