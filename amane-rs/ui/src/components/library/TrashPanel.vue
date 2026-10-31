<script setup lang="ts">
import { computed, onMounted, ref } from "vue";

import { api, describeError, type LibraryDto, type TrashPageDto } from "@/api/client";
import EmptyState from "@/components/common/EmptyState.vue";
import Alert from "@/components/ui/Alert.vue";
import Button from "@/components/ui/Button.vue";
import Paper from "@/components/ui/Paper.vue";
import { formatSize } from "@/lib/format";

const props = defineProps<{ library: LibraryDto }>();
const emit = defineEmits<{ notice: [tone: "error" | "ok", text: string] }>();

const listing = ref<TrashPageDto | null>(null);
const loading = ref(false);
const failure = ref<string | null>(null);
const restoring = ref<string | null>(null);

const items = computed(() => listing.value?.items ?? []);

async function load(): Promise<void> {
  loading.value = true;
  failure.value = null;
  try {
    listing.value = await api.trashListing(props.library.id);
  } catch (error) {
    listing.value = null;
    failure.value = describeError(error);
  } finally {
    loading.value = false;
  }
}

/** 恢复按原相对路径反向移动; 目标已存在时后端拒绝, 页面据此提示而不覆盖. */
async function restore(path: string): Promise<void> {
  restoring.value = path;
  try {
    listing.value = await api.trashRestore(props.library.id, path);
    emit("notice", "ok", `已恢复 ${path}`);
  } catch (error) {
    emit("notice", "error", describeError(error));
  } finally {
    restoring.value = null;
  }
}

onMounted(load);
</script>

<template>
  <div class="panel">
    <div class="panel__head">
      <p class="panel__hint">
        回收站是库内的 <code>.amane_trash</code>，占用由遍历现算。目标已存在时恢复会被拒绝。
      </p>
      <Button variant="default" size="sm" :disabled="loading" @click="load">刷新</Button>
    </div>

    <Alert v-if="failure !== null" tone="danger">
      {{ failure }}
      <template #action>
        <Button variant="subtle" size="sm" @click="load">重试</Button>
      </template>
    </Alert>

    <EmptyState v-else-if="loading && listing === null" title="读取中…" />

    <template v-else-if="listing !== null">
      <EmptyState
        v-if="items.length === 0"
        icon="trash"
        title="回收站是空的"
        description="清理与覆盖旧目标时移入的文件会出现在这里。"
      />

      <template v-else>
        <Paper padding="none">
          <table class="amane-table">
            <thead>
              <tr>
                <th>相对路径</th>
                <th style="width: 110px">大小</th>
                <th style="width: 120px"></th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="item in items" :key="item.path">
                <td class="mono">{{ item.path }}</td>
                <td class="mono dimmed">{{ formatSize(item.size) }}</td>
                <td class="cell-action">
                  <Button
                    variant="default"
                    size="sm"
                    :loading="restoring === item.path"
                    :disabled="restoring !== null"
                    @click="restore(item.path)"
                  >
                    恢复
                  </Button>
                </td>
              </tr>
            </tbody>
          </table>
        </Paper>
        <p class="panel__note">回收站占用：{{ formatSize(listing.total_size) }}。</p>
      </template>
    </template>
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

.panel__hint code {
  color: var(--amane-fg);
  font-family: var(--font-mono);
  font-size: 12px;
}

.panel__note {
  margin: 0;
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

.cell-action {
  text-align: right;
}
</style>
