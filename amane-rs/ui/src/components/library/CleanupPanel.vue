<script setup lang="ts">
import { computed, onMounted, ref } from "vue";

import { api, describeError, type CleanupManifestDto, type LibraryDto } from "@/api/client";
import EmptyState from "@/components/common/EmptyState.vue";
import Alert from "@/components/ui/Alert.vue";
import Badge from "@/components/ui/Badge.vue";
import Button from "@/components/ui/Button.vue";
import Modal from "@/components/ui/Modal.vue";
import Paper from "@/components/ui/Paper.vue";
import { formatSize } from "@/lib/format";
import { ruleLabel } from "@/lib/library/kinds";

const props = defineProps<{ library: LibraryDto }>();
const emit = defineEmits<{ notice: [tone: "error" | "ok", text: string] }>();

const manifest = ref<CleanupManifestDto | null>(null);
const loading = ref(false);
const failure = ref<string | null>(null);
const confirmOpen = ref(false);
const busy = ref(false);

const items = computed(() => manifest.value?.items ?? []);

async function load(): Promise<void> {
  loading.value = true;
  failure.value = null;
  try {
    manifest.value = await api.cleanupManifest(props.library.id);
  } catch (error) {
    manifest.value = null;
    failure.value = describeError(error);
  } finally {
    loading.value = false;
  }
}

async function execute(): Promise<void> {
  busy.value = true;
  try {
    const task = await api.cleanupExecute(props.library.id);
    confirmOpen.value = false;
    emit("notice", "ok", `清理任务 #${task.id} 已提交，完成后刷新查看结果`);
    await load();
  } catch (error) {
    emit("notice", "error", describeError(error));
  } finally {
    busy.value = false;
  }
}

onMounted(load);
</script>

<template>
  <div class="panel">
    <div class="panel__head">
      <p class="panel__hint">
        清理范围只含扫描索引里的无效文件与系统产物；执行时按索引重建清单，界面显示的与最终删除的内容同源。
      </p>
      <Button variant="default" size="sm" :disabled="loading" @click="load">刷新清单</Button>
    </div>

    <Alert v-if="failure !== null" tone="danger">
      {{ failure }}
      <template #action>
        <Button variant="subtle" size="sm" @click="load">重试</Button>
      </template>
    </Alert>

    <EmptyState v-else-if="loading && manifest === null" title="读取中…" />

    <template v-else-if="manifest !== null">
      <EmptyState
        v-if="items.length === 0"
        icon="check"
        title="没有待清理的文件"
        description="无效文件与系统产物会出现在这里。"
      />

      <template v-else>
        <Paper padding="none">
          <table class="amane-table">
            <thead>
              <tr>
                <th>相对路径</th>
                <th style="width: 110px">大小</th>
                <th style="width: 260px">命中原因</th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="item in items" :key="item.path">
                <td class="mono">{{ item.path }}</td>
                <td class="mono dimmed">{{ formatSize(item.size) }}</td>
                <td><Badge tone="warning" size="sm">{{ ruleLabel(item.reason) }}</Badge></td>
              </tr>
            </tbody>
          </table>
        </Paper>

        <div class="panel__foot">
          <Button variant="danger" size="sm" @click="confirmOpen = true">确认清理</Button>
          <span class="panel__note">
            共 {{ items.length }} 项，{{ formatSize(manifest.total_size) }}。清理的文件进入库内回收站，可恢复。
          </span>
        </div>
      </template>
    </template>

    <Modal :opened="confirmOpen" title="确认清理" :width="440" @close="confirmOpen = false">
      <p class="confirm-text">
        将把 {{ items.length }} 个文件（{{ formatSize(manifest?.total_size ?? 0) }}）移入库内回收站。
      </p>
      <p class="confirm-hint">原始归类会随索引删除，恢复后按「未识别」重新登记。</p>
      <template #footer>
        <Button variant="default" @click="confirmOpen = false">取消</Button>
        <Button variant="danger" :loading="busy" @click="execute">清理</Button>
      </template>
    </Modal>
  </div>
</template>

<style scoped>
.panel {
  display: flex;
  flex-direction: column;
  gap: 12px;
  min-width: 0;
}

.panel__head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
}

.panel__hint {
  margin: 0;
  color: var(--amane-dimmed);
  font-size: 13px;
  line-height: 1.6;
}

.panel__foot {
  display: flex;
  align-items: center;
  gap: 12px;
  flex-wrap: wrap;
}

.panel__note {
  color: var(--amane-subtle);
  font-size: 12px;
}

.mono {
  font-family: var(--font-mono);
  font-size: 12px;
  overflow-wrap: anywhere;
}

.dimmed {
  color: var(--amane-dimmed);
}

.confirm-text {
  margin: 0;
  font-size: 14px;
}

.confirm-hint {
  margin: 0;
  color: var(--amane-dimmed);
  font-size: 13px;
}
</style>
