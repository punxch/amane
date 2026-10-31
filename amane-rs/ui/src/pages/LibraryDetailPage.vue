<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { RouterLink, useRoute, useRouter } from "vue-router";

import { api, describeError, type LibraryDto } from "@/api/client";
import CleanupPanel from "@/components/library/CleanupPanel.vue";
import FilesPanel from "@/components/library/FilesPanel.vue";
import OrganizePanel from "@/components/library/OrganizePanel.vue";
import TrashPanel from "@/components/library/TrashPanel.vue";
import Alert from "@/components/ui/Alert.vue";
import Button from "@/components/ui/Button.vue";
import Modal from "@/components/ui/Modal.vue";
import PageShell from "@/components/common/PageShell.vue";
import { formatTime } from "@/lib/format";

type Tab = "files" | "organize" | "cleanup" | "trash";

const route = useRoute();
const router = useRouter();

const library = ref<LibraryDto | null>(null);
const loading = ref(false);
const failure = ref<string | null>(null);
const notice = ref<{ kind: "error" | "ok"; text: string } | null>(null);
const tab = ref<Tab>("files");

const scanning = ref(false);
const removeOpen = ref(false);
const removeBusy = ref(false);

const id = computed(() => {
  const raw = route.params.id;
  const parsed = Number.parseInt(typeof raw === "string" ? raw : "", 10);
  return Number.isNaN(parsed) ? null : parsed;
});

const fileTotal = computed(() => {
  const counts = library.value?.counts;
  if (counts === undefined) {
    return 0;
  }
  return Object.values(counts).reduce((sum, value) => sum + value, 0);
});

const subtitle = computed(() => {
  const item = library.value;
  if (item === null) {
    return "";
  }
  return `#${item.id} · 登记于 ${formatTime(item.created_at)} · ${fileTotal.value} 条索引记录`;
});

const tabs = computed<{ key: Tab; label: string }[]>(() => [
  { key: "files", label: "文件" },
  { key: "organize", label: "整理预览" },
  { key: "cleanup", label: "清理" },
  { key: "trash", label: "回收站" },
]);

function report(kind: "error" | "ok", text: string): void {
  notice.value = { kind, text };
}

async function load(): Promise<void> {
  if (id.value === null) {
    failure.value = "地址里的媒体库编号不是整数";
    return;
  }
  loading.value = true;
  failure.value = null;
  try {
    const page = await api.listLibraries();
    const found = page.items.find((item) => item.id === id.value) ?? null;
    library.value = found;
    failure.value = found === null ? `媒体库 #${id.value} 不在登记表里` : null;
  } catch (error) {
    library.value = null;
    failure.value = describeError(error);
  } finally {
    loading.value = false;
  }
}

async function scan(): Promise<void> {
  const item = library.value;
  if (item === null) {
    return;
  }
  scanning.value = true;
  try {
    const task = await api.scanLibrary(item.id);
    report("ok", `扫描任务 #${task.id} 已提交，完成后刷新查看结果`);
    await load();
  } catch (error) {
    report("error", describeError(error));
  } finally {
    scanning.value = false;
  }
}

async function remove(): Promise<void> {
  const item = library.value;
  if (item === null) {
    return;
  }
  removeBusy.value = true;
  try {
    await api.deleteLibrary(item.id);
    await router.replace({ name: "libraries" });
  } catch (error) {
    report("error", describeError(error));
  } finally {
    removeBusy.value = false;
  }
}

watch(id, () => void load(), { immediate: true });
</script>

<template>
  <PageShell :title="library?.path ?? '媒体库'" :subtitle="subtitle">
    <template #actions>
      <RouterLink to="/libraries">
        <Button variant="subtle" size="sm">返回列表</Button>
      </RouterLink>
      <Button variant="default" :loading="scanning" :disabled="library === null" @click="scan">
        扫描
      </Button>
      <Button
        variant="danger"
        size="sm"
        :disabled="library === null"
        @click="removeOpen = true"
      >
        移除
      </Button>
    </template>

    <Alert v-if="notice !== null" :tone="notice.kind === 'error' ? 'danger' : 'success'">
      {{ notice.text }}
    </Alert>

    <Alert v-if="failure !== null" tone="danger">
      {{ failure }}
      <template #action>
        <Button variant="subtle" size="sm" @click="load">重试</Button>
      </template>
    </Alert>

    <template v-else-if="library !== null">
      <div class="tabs" role="tablist">
        <button
          v-for="entry in tabs"
          :key="entry.key"
          type="button"
          role="tab"
          class="tabs__item"
          :data-active="tab === entry.key || undefined"
          :aria-selected="tab === entry.key"
          @click="tab = entry.key"
        >
          {{ entry.label }}
        </button>
      </div>

      <FilesPanel
        v-if="tab === 'files'"
        :key="`files-${library.id}`"
        :library="library"
        @notice="report"
      />
      <OrganizePanel
        v-else-if="tab === 'organize'"
        :key="`organize-${library.id}`"
        :library="library"
        @notice="report"
      />
      <CleanupPanel
        v-else-if="tab === 'cleanup'"
        :key="`cleanup-${library.id}`"
        :library="library"
        @notice="report"
      />
      <TrashPanel
        v-else
        :key="`trash-${library.id}`"
        :library="library"
        @notice="report"
      />
    </template>

    <Modal :opened="removeOpen" title="移除登记" :width="440" @close="removeOpen = false">
      <p class="confirm-text">
        移除 <strong>{{ library?.path }}</strong> 的登记与扫描索引。
      </p>
      <p class="confirm-hint">磁盘上的文件不受影响，可以重新登记。</p>
      <template #footer>
        <Button variant="default" @click="removeOpen = false">取消</Button>
        <Button variant="danger" :loading="removeBusy" @click="remove">移除</Button>
      </template>
    </Modal>
  </PageShell>
</template>

<style scoped>
/* 标题是库的绝对路径: 等宽字体让斜杠与数码可读, 长路径允许折行. */
:deep(.amane-page__title h1) {
  font-family: var(--font-mono);
  font-size: 18px;
  overflow-wrap: anywhere;
}

.tabs {
  display: flex;
  gap: 4px;
  border-bottom: 1px solid var(--amane-line);
  margin-bottom: 16px;
}

.tabs__item {
  position: relative;
  height: 36px;
  padding: 0 12px;
  border: 0;
  background: transparent;
  color: var(--amane-dimmed);
  font-size: 14px;
  cursor: pointer;
}

.tabs__item:hover {
  color: var(--amane-bright);
}

.tabs__item[data-active="true"] {
  color: var(--amane-bright);
  font-weight: 600;
}

.tabs__item[data-active="true"]::after {
  content: "";
  position: absolute;
  right: 8px;
  bottom: -1px;
  left: 8px;
  height: 2px;
  border-radius: 2px;
  background: var(--amane-brand);
}

.confirm-text {
  margin: 0;
  font-size: 14px;
  overflow-wrap: anywhere;
}

.confirm-hint {
  margin: 0;
  color: var(--amane-dimmed);
  font-size: 13px;
}
</style>
