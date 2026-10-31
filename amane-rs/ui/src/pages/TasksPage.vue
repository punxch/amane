<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { useRoute, useRouter, type LocationQueryValue } from "vue-router";

import { api, describeError, type TaskDto, type TaskLogEntry } from "@/api/client";
import EmptyState from "@/components/common/EmptyState.vue";
import PageShell from "@/components/common/PageShell.vue";
import Pagination from "@/components/common/Pagination.vue";
import Alert from "@/components/ui/Alert.vue";
import Badge from "@/components/ui/Badge.vue";
import Button from "@/components/ui/Button.vue";
import Icon from "@/components/ui/Icon.vue";
import Paper from "@/components/ui/Paper.vue";
import SelectInput from "@/components/ui/SelectInput.vue";
import TextInput from "@/components/ui/TextInput.vue";
import { formatClock, formatTime, pageCount } from "@/lib/format";
import {
  TASK_STATUS_FILTERS,
  isRetryableStatus,
  isTerminalStatus,
  taskKind,
  taskStatus,
} from "@/lib/task/status";

const PAGE_SIZE = 50;
const POLL_INTERVAL_MS = 2000;

const route = useRoute();
const router = useRouter();

const tasks = ref<TaskDto[]>([]);
const total = ref(0);
const loading = ref(false);
const failure = ref<string | null>(null);
const notice = ref<{ kind: "error" | "ok"; text: string } | null>(null);

const selectedSnapshot = ref<TaskDto | null>(null);
const logLines = ref<TaskLogEntry[]>([]);
const children = ref<TaskDto[]>([]);
const numberInput = ref("");
const busy = ref(false);

let timer: number | undefined;

function readQuery(value: LocationQueryValue | LocationQueryValue[] | undefined): string {
  return typeof value === "string" ? value : "";
}

const status = computed(() => readQuery(route.query.status));
const page = computed(() => {
  const parsed = Number.parseInt(readQuery(route.query.page), 10);
  return Number.isNaN(parsed) || parsed < 1 ? 1 : parsed;
});
const pages = computed(() => pageCount(total.value, PAGE_SIZE));

/** 选中项写进 URL: 刷新与分享后仍停在同一条任务上. */
const selectedId = computed<number | null>(() => {
  const raw = readQuery(route.query.task);
  if (raw === "") {
    return null;
  }
  const parsed = Number.parseInt(raw, 10);
  return Number.isNaN(parsed) ? null : parsed;
});

const subtitle = computed(() => {
  const count = loading.value ? "读取中" : `${total.value} 条`;
  return status.value === "" ? count : `${count} · 已按状态筛选`;
});

/** 轮询只替换列表里的同一条任务, 选中项因此跟着状态一起更新. */
const activeTask = computed(() => {
  const id = selectedId.value;
  if (id === null) {
    return null;
  }
  return tasks.value.find((task) => task.id === id) ?? selectedSnapshot.value;
});

const statusFilters = TASK_STATUS_FILTERS.map((entry) => ({
  value: entry.value,
  label: entry.label,
}));

function report(kind: "error" | "ok", text: string): void {
  notice.value = { kind, text };
}

function currentQuery(): Record<string, string> {
  const query: Record<string, string> = {};
  for (const [key, value] of Object.entries(route.query)) {
    if (typeof value === "string" && value !== "") {
      query[key] = value;
    }
  }
  return query;
}

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

async function refresh(): Promise<void> {
  try {
    const result = await api.listTasks(
      status.value === "" ? null : status.value,
      PAGE_SIZE,
      (page.value - 1) * PAGE_SIZE,
    );
    tasks.value = result.items;
    total.value = result.total;
    failure.value = null;
  } catch (error) {
    failure.value = describeError(error);
  }
}

async function loadLog(id: number): Promise<void> {
  try {
    logLines.value = (await api.taskLog(id)).items;
  } catch (error) {
    notice.value = { kind: "error", text: describeError(error) };
  }
}

async function loadChildren(id: number): Promise<void> {
  try {
    children.value = (await api.taskChildren(id)).items;
  } catch (error) {
    notice.value = { kind: "error", text: describeError(error) };
  }
}

/**
 * 按 URL 里的标识载入详情.
 *
 * 任务可能不在当前页或当前筛选里, 因此列表找不到时单独取一条; 两条路径得到同一个 `TaskDto`.
 */
async function loadSelection(id: number | null): Promise<void> {
  if (id === null) {
    selectedSnapshot.value = null;
    logLines.value = [];
    children.value = [];
    return;
  }

  const found = tasks.value.find((task) => task.id === id) ?? null;
  if (found !== null) {
    selectedSnapshot.value = found;
  } else {
    try {
      selectedSnapshot.value = await api.task(id);
    } catch (error) {
      selectedSnapshot.value = null;
      report("error", describeError(error));
    }
  }
  await Promise.all([loadLog(id), loadChildren(id)]);
}

function select(task: TaskDto): void {
  updateQuery({ task: String(task.id) });
}

async function submitScrape(): Promise<void> {
  const number = numberInput.value.trim();
  if (number === "") {
    report("error", "请填写番号");
    return;
  }
  busy.value = true;
  try {
    const task = await api.createTask({ kind: "scrape", payload: { number }, priority: 0 });
    numberInput.value = "";
    report("ok", `已提交任务 #${task.id}`);
    await refresh();
    select(task);
  } catch (error) {
    report("error", describeError(error));
  } finally {
    busy.value = false;
  }
}

async function cancelTask(id: number): Promise<void> {
  busy.value = true;
  try {
    await api.cancelTask(id);
    await refresh();
  } catch (error) {
    report("error", describeError(error));
  } finally {
    busy.value = false;
  }
}

async function retryTask(id: number): Promise<void> {
  busy.value = true;
  try {
    const task = await api.retryTask(id);
    await refresh();
    select(task);
  } catch (error) {
    report("error", describeError(error));
  } finally {
    busy.value = false;
  }
}

watch([status, page], () => void refresh());
watch(selectedId, (id) => void loadSelection(id), { immediate: true });

onMounted(async () => {
  await refresh();
  timer = window.setInterval(() => {
    void refresh();
    const current = activeTask.value;
    if (current !== null) {
      void loadLog(current.id);
      void loadChildren(current.id);
    }
  }, POLL_INTERVAL_MS);
});

onBeforeUnmount(() => {
  if (timer !== undefined) {
    window.clearInterval(timer);
  }
});
</script>

<template>
  <PageShell title="任务" :subtitle="subtitle">
    <template #actions>
      <Button variant="default" :disabled="loading" @click="refresh">刷新</Button>
    </template>

    <template #toolbar>
      <SelectInput
        :model-value="status"
        :options="statusFilters"
        placeholder="全部状态"
        aria-label="状态"
        @update:model-value="(value) => updateQuery({ status: value, page: null })"
      />
      <form class="scrape-form" @submit.prevent="submitScrape">
        <TextInput
          v-model="numberInput"
          mono
          placeholder="SSIS001"
          aria-label="番号"
          class="scrape-input"
        />
        <Button type="submit" :loading="busy">提交刮削</Button>
      </form>
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

    <div v-else class="tasks">
      <div class="tasks__list">
        <EmptyState
          v-if="!loading && tasks.length === 0"
          icon="list"
          title="还没有任务"
          description="在上方填写番号提交一次刮削。"
        />

        <Paper v-else padding="none">
          <table class="amane-table">
            <thead>
              <tr>
                <th style="width: 70px">#</th>
                <th>类型</th>
                <th style="width: 100px">状态</th>
                <th style="width: 70px">尝试</th>
                <th style="width: 170px">创建时间</th>
                <th style="width: 170px">结束时间</th>
              </tr>
            </thead>
            <tbody>
              <tr
                v-for="task in tasks"
                :key="task.id"
                data-clickable="true"
                :data-selected="selectedId === task.id || undefined"
                @click="select(task)"
              >
                <td class="mono dimmed">{{ task.id }}</td>
                <td>
                  <span class="kind">{{ taskKind(task.kind) }}</span>
                  <Badge v-if="task.schedule_id !== null" tone="gray" size="sm" class="schedule">
                    定时任务 #{{ task.schedule_id }}
                  </Badge>
                </td>
                <td>
                  <Badge :tone="taskStatus(task.status).tone" size="sm">
                    {{ taskStatus(task.status).label }}
                  </Badge>
                </td>
                <td class="mono dimmed">{{ task.attempts }}</td>
                <td class="mono dimmed small nowrap">{{ formatTime(task.created_at) }}</td>
                <td class="mono dimmed small nowrap">{{ formatTime(task.finished_at) }}</td>
              </tr>
            </tbody>
          </table>
        </Paper>

        <div v-if="pages > 1" class="tasks__pagination">
          <Pagination
            :total-pages="pages"
            :page="page"
            @change="(value) => updateQuery({ page: String(value) })"
          />
        </div>
      </div>

      <aside class="tasks__detail">
        <Paper v-if="activeTask !== null">
          <div class="detail-head">
            <h2>任务 #{{ activeTask.id }}</h2>
            <Badge :tone="taskStatus(activeTask.status).tone" size="sm">
              {{ taskStatus(activeTask.status).label }}
            </Badge>
          </div>

          <dl class="facts">
            <div>
              <dt>类型</dt>
              <dd>{{ taskKind(activeTask.kind) }}</dd>
            </div>
            <div>
              <dt>优先级</dt>
              <dd>{{ activeTask.priority }}</dd>
            </div>
            <div>
              <dt>重试自</dt>
              <dd>{{ activeTask.retry_of ?? "—" }}</dd>
            </div>
            <div>
              <dt>开始</dt>
              <dd>{{ formatTime(activeTask.started_at) }}</dd>
            </div>
          </dl>

          <Alert v-if="activeTask.error" tone="danger" class="detail-error">
            {{ activeTask.error }}
          </Alert>

          <div class="detail-actions">
            <Button
              v-if="!isTerminalStatus(activeTask.status)"
              variant="default"
              size="sm"
              :disabled="busy"
              @click="cancelTask(activeTask.id)"
            >
              取消
            </Button>
            <Button
              v-if="isRetryableStatus(activeTask.status)"
              size="sm"
              :disabled="busy"
              @click="retryTask(activeTask.id)"
            >
              重试
            </Button>
          </div>

          <template v-if="children.length > 0">
            <h3 class="section-title">子任务</h3>
            <ul class="children">
              <li v-for="child in children" :key="child.id">
                <span class="mono">#{{ child.id }} {{ taskKind(child.kind) }}</span>
                <Badge :tone="taskStatus(child.status).tone" size="sm">
                  {{ taskStatus(child.status).label }}
                </Badge>
              </li>
            </ul>
          </template>

          <div class="section-head">
            <h3 class="section-title">日志</h3>
            <Button variant="subtle" size="sm" @click="loadLog(activeTask.id)">刷新</Button>
          </div>
          <ol class="log">
            <li v-for="(line, index) in logLines" :key="index" :data-level="line.level">
              <span class="log__time">{{ formatClock(line.at) }}</span>
              <span class="log__message">{{ line.message }}</span>
            </li>
            <li v-if="logLines.length === 0" class="log__empty">暂无日志</li>
          </ol>
        </Paper>

        <Paper v-else class="tasks__hint">
          <Icon name="list" :size="20" :stroke="1.4" />
          <p>选择一条任务查看日志与子任务。</p>
        </Paper>
      </aside>
    </div>
  </PageShell>
</template>

<style scoped>
.scrape-form {
  display: flex;
  align-items: center;
  gap: 8px;
}

.scrape-input {
  width: 160px;
}

.tasks {
  display: grid;
  grid-template-columns: minmax(0, 1fr);
  gap: 16px;
  align-items: start;
}

@media (min-width: 62em) {
  .tasks {
    grid-template-columns: minmax(0, 1fr) 360px;
  }
}

.tasks__list {
  display: flex;
  flex-direction: column;
  gap: 12px;
  min-width: 0;
}

.tasks__pagination {
  padding-top: 4px;
}

.tasks__detail {
  min-width: 0;
}

.tasks__hint {
  display: flex;
  align-items: center;
  gap: 10px;
  color: var(--amane-dimmed);
  font-size: 13px;
}

.tasks__hint p {
  margin: 0;
}

.kind {
  color: var(--amane-fg);
}

.schedule {
  margin-left: 6px;
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

.nowrap {
  white-space: nowrap;
}

.detail-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  margin-bottom: 12px;
}

.detail-head h2 {
  font-size: 15px;
}

.facts {
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin: 0;
}

.facts > div {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 12px;
}

.facts dt {
  color: var(--amane-dimmed);
  font-size: 13px;
}

.facts dd {
  margin: 0;
  color: var(--amane-fg);
  font-family: var(--font-mono);
  font-size: 12px;
}

.detail-error {
  margin-top: 12px;
}

.detail-actions {
  display: flex;
  gap: 8px;
  margin-top: 14px;
}

.section-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  margin-top: 16px;
}

.section-title {
  margin: 16px 0 6px;
  color: var(--amane-dimmed);
  font-size: 12px;
  font-weight: 600;
}

.section-head .section-title {
  margin: 0;
}

.children {
  display: flex;
  flex-direction: column;
  gap: 4px;
  margin: 0;
  padding: 0;
  list-style: none;
}

.children li {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  font-size: 12px;
}

.log {
  display: flex;
  flex-direction: column;
  gap: 3px;
  max-height: 320px;
  margin: 0;
  padding: 0;
  overflow-y: auto;
  list-style: none;
}

.log li {
  display: flex;
  gap: 8px;
  font-family: var(--font-mono);
  font-size: 12px;
  line-height: 1.5;
}

.log__time {
  flex-shrink: 0;
  color: var(--amane-subtle);
}

.log__message {
  color: var(--amane-fg);
  overflow-wrap: anywhere;
}

.log li[data-level="error"] .log__message {
  color: var(--amane-red);
}

.log li[data-level="warn"] .log__message {
  color: var(--amane-yellow);
}

.log__empty {
  color: var(--amane-subtle);
}
</style>
