<script setup lang="ts">
import { onMounted, ref } from "vue";
import { RouterLink } from "vue-router";

import { api, describeError, type LibraryDto } from "@/api/client";
import EmptyState from "@/components/common/EmptyState.vue";
import PageShell from "@/components/common/PageShell.vue";
import Alert from "@/components/ui/Alert.vue";
import Badge from "@/components/ui/Badge.vue";
import Button from "@/components/ui/Button.vue";
import Modal from "@/components/ui/Modal.vue";
import Paper from "@/components/ui/Paper.vue";
import TextInput from "@/components/ui/TextInput.vue";
import { formatTime } from "@/lib/format";
import { SCAN_KINDS } from "@/lib/library/kinds";

const libraries = ref<LibraryDto[]>([]);
const loading = ref(false);
const failure = ref<string | null>(null);
const notice = ref<{ kind: "error" | "ok"; text: string } | null>(null);

const createOpen = ref(false);
const pathDraft = ref("");
const creating = ref(false);

const scanning = ref<number | null>(null);
const removing = ref<LibraryDto | null>(null);
const removeBusy = ref(false);

function report(kind: "error" | "ok", text: string): void {
  notice.value = { kind, text };
}

async function refresh(): Promise<void> {
  loading.value = true;
  failure.value = null;
  try {
    libraries.value = (await api.listLibraries()).items;
  } catch (error) {
    failure.value = describeError(error);
  } finally {
    loading.value = false;
  }
}

function countOf(library: LibraryDto, kind: string): number {
  return library.counts[kind] ?? 0;
}

function scannedTotal(library: LibraryDto): number {
  return Object.values(library.counts).reduce((sum, value) => sum + value, 0);
}

async function create(): Promise<void> {
  const path = pathDraft.value.trim();
  if (path === "") {
    report("error", "请填写目录路径");
    return;
  }
  creating.value = true;
  try {
    await api.createLibrary({ path });
    createOpen.value = false;
    pathDraft.value = "";
    report("ok", "媒体库已登记，扫描需要手动触发");
    await refresh();
  } catch (error) {
    report("error", describeError(error));
  } finally {
    creating.value = false;
  }
}

async function scan(library: LibraryDto): Promise<void> {
  scanning.value = library.id;
  try {
    const task = await api.scanLibrary(library.id);
    report("ok", `扫描任务 #${task.id} 已提交，完成后刷新查看结果`);
    await refresh();
  } catch (error) {
    report("error", describeError(error));
  } finally {
    scanning.value = null;
  }
}

async function remove(): Promise<void> {
  const target = removing.value;
  if (target === null) {
    return;
  }
  removeBusy.value = true;
  try {
    await api.deleteLibrary(target.id);
    removing.value = null;
    report("ok", "已移除登记；磁盘文件不受影响");
    await refresh();
  } catch (error) {
    report("error", describeError(error));
  } finally {
    removeBusy.value = false;
  }
}

onMounted(refresh);
</script>

<template>
  <PageShell title="媒体库" :subtitle="loading ? '读取中' : `${libraries.length} 个库`">
    <template #actions>
      <Button variant="default" :disabled="loading" @click="refresh">刷新</Button>
      <Button @click="createOpen = true">登记媒体库</Button>
    </template>

    <Alert v-if="notice !== null" :tone="notice.kind === 'error' ? 'danger' : 'success'">
      {{ notice.text }}
    </Alert>

    <Alert v-if="failure !== null" tone="danger">
      {{ failure }}
      <template #action>
        <Button variant="subtle" size="sm" @click="refresh">重试</Button>
      </template>
    </Alert>

    <EmptyState
      v-else-if="!loading && libraries.length === 0"
      icon="folder"
      title="还没有登记媒体库"
      description="登记一个目录后可以扫描归类，整理与清理都以扫描索引为准。"
    >
      <Button size="sm" @click="createOpen = true">登记媒体库</Button>
    </EmptyState>

    <Paper v-else padding="none">
      <table class="amane-table">
        <thead>
          <tr>
            <th style="width: 64px">编号</th>
            <th>路径</th>
            <th style="width: 320px">归类</th>
            <th style="width: 170px">登记时间</th>
            <th style="width: 200px"></th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="library in libraries" :key="library.id">
            <td class="mono dimmed">#{{ library.id }}</td>
            <td>
              <RouterLink
                :to="{ name: 'library-detail', params: { id: String(library.id) } }"
                class="library-path"
              >
                {{ library.path }}
              </RouterLink>
            </td>
            <td>
              <div v-if="scannedTotal(library) === 0" class="dimmed small">尚未扫描</div>
              <div v-else class="kind-badges">
                <Badge
                  v-for="kind in SCAN_KINDS"
                  v-show="countOf(library, kind.value) > 0"
                  :key="kind.value"
                  :tone="kind.value === 'unwanted' ? 'danger' : 'gray'"
                  size="sm"
                >
                  {{ kind.label }} {{ countOf(library, kind.value) }}
                </Badge>
              </div>
            </td>
            <td class="mono dimmed small">{{ formatTime(library.created_at) }}</td>
            <td>
              <div class="row-actions">
                <Button
                  variant="default"
                  size="sm"
                  :loading="scanning === library.id"
                  :disabled="scanning !== null"
                  @click="scan(library)"
                >
                  扫描
                </Button>
                <RouterLink :to="{ name: 'library-detail', params: { id: String(library.id) } }">
                  <Button variant="subtle" size="sm">打开</Button>
                </RouterLink>
                <Button variant="subtle" size="sm" @click="removing = library">移除</Button>
              </div>
            </td>
          </tr>
        </tbody>
      </table>
    </Paper>

    <Modal :opened="createOpen" title="登记媒体库" :width="520" @close="createOpen = false">
      <p class="modal-hint">路径必须存在且可读；登记后点「扫描」归类文件。</p>
      <label class="field">
        <span>目录路径</span>
        <TextInput v-model="pathDraft" mono placeholder="/Volumes/media/library" />
      </label>
      <template #footer>
        <Button variant="default" @click="createOpen = false">取消</Button>
        <Button :loading="creating" @click="create">登记</Button>
      </template>
    </Modal>

    <Modal :opened="removing !== null" title="移除登记" :width="440" @close="removing = null">
      <p class="modal-hint">
        移除 <strong>{{ removing?.path }}</strong> 的登记与扫描索引。磁盘上的文件不受影响，可以重新登记。
      </p>
      <template #footer>
        <Button variant="default" @click="removing = null">取消</Button>
        <Button variant="danger" :loading="removeBusy" @click="remove">移除</Button>
      </template>
    </Modal>
  </PageShell>
</template>

<style scoped>
.library-path {
  color: var(--amane-fg);
  font-family: var(--font-mono);
  font-size: 13px;
  overflow-wrap: anywhere;
}

.library-path:hover {
  color: var(--amane-brand-text);
}

.kind-badges {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
}

.row-actions {
  display: flex;
  justify-content: flex-end;
  gap: 6px;
}

.mono {
  font-family: var(--font-mono);
}

.dimmed {
  color: var(--amane-dimmed);
}

.small {
  font-size: 12px;
}

.modal-hint {
  margin: 0;
  color: var(--amane-dimmed);
  font-size: 13px;
  line-height: 1.6;
}

.field {
  display: flex;
  flex-direction: column;
  gap: 6px;
  color: var(--amane-dimmed);
  font-size: 12px;
}
</style>
