<script setup lang="ts">
import { computed, onMounted, ref } from "vue";

import { api, describeError, type LibraryDto, type OrganizePreviewDto } from "@/api/client";
import EmptyState from "@/components/common/EmptyState.vue";
import Alert from "@/components/ui/Alert.vue";
import Badge from "@/components/ui/Badge.vue";
import Button from "@/components/ui/Button.vue";
import Paper from "@/components/ui/Paper.vue";

const props = defineProps<{ library: LibraryDto }>();
const emit = defineEmits<{ notice: [tone: "error" | "ok", text: string] }>();

type ConflictChoice = "skip" | "rename" | "overwrite";

const CONFLICT_LABELS: Record<string, string> = {
  exists: "目标已存在",
  number_mismatch: "同名不同番号",
  same_path: "源与目标相同",
};

const preview = ref<OrganizePreviewDto | null>(null);
const loading = ref(false);
const executing = ref<ConflictChoice | null>(null);
const failure = ref<string | null>(null);

const conflicts = computed(() => preview.value?.items ?? []);

function conflictLabel(kind: string): string {
  return CONFLICT_LABELS[kind] ?? kind;
}

async function load(): Promise<void> {
  loading.value = true;
  failure.value = null;
  try {
    preview.value = await api.organizePreview(props.library.id);
  } catch (error) {
    preview.value = null;
    failure.value = describeError(error);
  } finally {
    loading.value = false;
  }
}

/**
 * 执行整理. 冲突项统一按所选处置处理, 无冲突项照常移动;
 * 目标路径由后端按当前配置重新渲染, 与预览同一套逻辑.
 */
async function execute(choice: ConflictChoice): Promise<void> {
  if (preview.value === null) {
    return;
  }
  executing.value = choice;
  try {
    const choices: Record<string, string> = {};
    for (const item of preview.value.items) {
      if (item.conflict !== null && item.conflict !== undefined) {
        choices[item.source_path] = choice;
      }
    }
    const task = await api.organizeExecute(props.library.id, choices);
    emit("notice", "ok", `整理任务 #${task.id} 已提交，完成后刷新查看结果`);
    await load();
  } catch (error) {
    emit("notice", "error", describeError(error));
  } finally {
    executing.value = null;
  }
}

onMounted(load);
</script>

<template>
  <div class="panel">
    <div class="panel__head">
      <p class="panel__hint">
        目标路径按当前模板渲染；预览不创建目录、不移动文件，重复预览结果一致。
      </p>
      <Button variant="default" size="sm" :disabled="loading" @click="load">重新预览</Button>
    </div>

    <Alert v-if="failure !== null" tone="danger">
      {{ failure }}
      <template #action>
        <Button variant="subtle" size="sm" @click="load">重试</Button>
      </template>
    </Alert>

    <EmptyState v-else-if="loading && preview === null" title="预览计算中…" />

    <template v-else-if="preview !== null">
      <Alert v-if="preview.unmatched > 0 || preview.stale > 0" tone="warning">
        <template v-if="preview.unmatched > 0">
          {{ preview.unmatched }} 个文件提取不出番号，未参与整理。
        </template>
        <template v-if="preview.stale > 0">
          {{ preview.stale }} 个索引记录在磁盘上已不存在。
        </template>
      </Alert>

      <EmptyState
        v-if="conflicts.length === 0"
        icon="arrow"
        title="没有可整理的文件"
        description="先扫描，再确认文件名里带番号。"
      />

      <Paper v-else padding="none">
        <table class="amane-table">
          <thead>
            <tr>
              <th>源路径</th>
              <th style="width: 40px"></th>
              <th>目标路径</th>
              <th style="width: 130px">冲突</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="item in conflicts" :key="item.source_path">
              <td class="mono dimmed">{{ item.source_path }}</td>
              <td class="arrow">→</td>
              <td class="mono">{{ item.target_path }}</td>
              <td>
                <Badge
                  v-if="item.conflict"
                  :tone="item.conflict === 'same_path' ? 'gray' : 'warning'"
                  size="sm"
                >
                  {{ conflictLabel(item.conflict) }}
                </Badge>
                <span v-else class="dimmed">—</span>
              </td>
            </tr>
          </tbody>
        </table>
      </Paper>

      <div v-if="conflicts.length > 0" class="panel__foot">
        <div class="panel__actions">
          <Button
            variant="default"
            size="sm"
            :loading="executing === 'skip'"
            :disabled="executing !== null"
            @click="execute('skip')"
          >
            执行（跳过冲突）
          </Button>
          <Button
            variant="default"
            size="sm"
            :loading="executing === 'rename'"
            :disabled="executing !== null"
            @click="execute('rename')"
          >
            执行（冲突改名）
          </Button>
          <Button
            variant="default"
            size="sm"
            :loading="executing === 'overwrite'"
            :disabled="executing !== null"
            @click="execute('overwrite')"
          >
            执行（覆盖冲突）
          </Button>
        </div>
        <p class="panel__note">覆盖前旧文件进入库内回收站，可在回收站恢复。</p>
      </div>
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

.panel__foot {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.panel__actions {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
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

.arrow {
  color: var(--amane-subtle);
  text-align: center;
}
</style>
