<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";

import {
  ApiError,
  api,
  type TaskDto,
  type TaskLogEntry,
} from "@/api/client";
import Button from "@/components/ui/Button.vue";

const STATUS_LABELS: Record<string, string> = {
  queued: "排队",
  running: "运行中",
  done: "完成",
  failed: "失败",
  cancelled: "已取消",
};

const POLL_INTERVAL_MS = 2000;

const tasks = ref<TaskDto[]>([]);
const total = ref(0);
const statusFilter = ref("");
const selected = ref<TaskDto | null>(null);
const logLines = ref<TaskLogEntry[]>([]);
const children = ref<TaskDto[]>([]);
const notice = ref<string | null>(null);
const numberInput = ref("");
const busy = ref(false);

let timer: number | undefined;

const activeTask = computed(() =>
  selected.value === null
    ? null
    : (tasks.value.find((task) => task.id === selected.value?.id) ?? selected.value),
);

function describe(error: unknown): string {
  return error instanceof ApiError ? error.message : "请求失败";
}

function statusLabel(status: string): string {
  return STATUS_LABELS[status] ?? status;
}

function statusClass(status: string): string {
  switch (status) {
    case "running":
      return "text-amber-400";
    case "done":
      return "text-emerald-400";
    case "failed":
      return "text-red-400";
    default:
      return "text-neutral-500";
  }
}

function formatTime(value: string | null | undefined): string {
  return value === null || value === undefined
    ? "—"
    : new Date(value).toLocaleString("zh-CN", { hour12: false });
}

async function refresh(): Promise<void> {
  try {
    const page = await api.listTasks(statusFilter.value === "" ? null : statusFilter.value);
    tasks.value = page.items;
    total.value = page.total;
  } catch (error) {
    notice.value = describe(error);
  }
}

async function loadLog(id: number): Promise<void> {
  try {
    logLines.value = (await api.taskLog(id)).items;
  } catch (error) {
    notice.value = describe(error);
  }
}

async function loadChildren(id: number): Promise<void> {
  try {
    children.value = (await api.taskChildren(id)).items;
  } catch (error) {
    notice.value = describe(error);
  }
}

async function select(task: TaskDto): Promise<void> {
  selected.value = task;
  await Promise.all([loadLog(task.id), loadChildren(task.id)]);
}

async function submitScrape(): Promise<void> {
  const number = numberInput.value.trim();
  if (number === "") {
    notice.value = "请填写番号";
    return;
  }
  busy.value = true;
  try {
    const task = await api.createTask({ kind: "scrape", payload: { number }, priority: 0 });
    numberInput.value = "";
    notice.value = `已提交任务 ${task.id}`;
    await refresh();
    await select(task);
  } catch (error) {
    notice.value = describe(error);
  } finally {
    busy.value = false;
  }
}

async function cancelTask(id: number): Promise<void> {
  try {
    await api.cancelTask(id);
    await refresh();
  } catch (error) {
    notice.value = describe(error);
  }
}

async function retryTask(id: number): Promise<void> {
  try {
    const task = await api.retryTask(id);
    await refresh();
    await select(task);
  } catch (error) {
    notice.value = describe(error);
  }
}

function isTerminal(status: string): boolean {
  return status === "done" || status === "failed" || status === "cancelled";
}

function isRetryable(status: string): boolean {
  return status === "failed" || status === "cancelled";
}

onMounted(async () => {
  await refresh();
  timer = window.setInterval(() => {
    void refresh();
    const id = activeTask.value?.id;
    if (id !== undefined) {
      void loadLog(id);
    }
  }, POLL_INTERVAL_MS);
});

onUnmounted(() => {
  if (timer !== undefined) {
    window.clearInterval(timer);
  }
});
</script>

<template>
  <div>
    <div class="flex items-center justify-between px-8 pt-5">
      <div class="flex items-center gap-3">
        <select
          v-model="statusFilter"
          class="rounded-md border border-neutral-800 bg-neutral-900 px-2 py-1 text-xs text-neutral-300 outline-none"
          @change="refresh"
        >
          <option value="">全部状态</option>
          <option value="queued">排队</option>
          <option value="running">运行中</option>
          <option value="done">完成</option>
          <option value="failed">失败</option>
          <option value="cancelled">已取消</option>
        </select>
        <span class="font-mono text-xs text-neutral-500">{{ total }} 条</span>
      </div>
      <Button variant="ghost" @click="refresh">刷新</Button>
    </div>

    <p
      v-if="notice"
      class="mx-8 mt-4 rounded-md border border-neutral-800 bg-neutral-900/60 px-4 py-2 text-sm text-neutral-300"
    >
      {{ notice }}
    </p>

    <main class="grid gap-6 px-8 py-6 lg:grid-cols-[minmax(0,1fr)_360px]">
      <section class="overflow-hidden rounded-lg border border-neutral-800/80">
        <div class="flex items-center gap-2 border-b border-neutral-800/80 px-4 py-3">
          <input
            v-model="numberInput"
            class="w-40 rounded-md border border-neutral-800 bg-neutral-900 px-3 py-1 font-mono text-sm text-neutral-100 outline-none focus:border-emerald-600"
            placeholder="SSIS001"
            @keyup.enter="submitScrape"
          />
          <Button :disabled="busy" @click="submitScrape">提交刮削</Button>
        </div>

        <table class="w-full border-collapse text-sm">
          <thead>
            <tr class="border-b border-neutral-800/80 text-left text-xs text-neutral-500">
              <th class="px-4 py-2 font-normal">#</th>
              <th class="px-4 py-2 font-normal">类型</th>
              <th class="px-4 py-2 font-normal">状态</th>
              <th class="px-4 py-2 font-normal">尝试</th>
              <th class="px-4 py-2 font-normal">创建时间</th>
              <th class="px-4 py-2 font-normal">结束时间</th>
            </tr>
          </thead>
          <tbody>
            <tr
              v-for="task in tasks"
              :key="task.id"
              class="cursor-pointer border-b border-neutral-900 last:border-0 hover:bg-neutral-900/40"
              :class="selected?.id === task.id ? 'bg-neutral-900/60' : ''"
              @click="select(task)"
            >
              <td class="px-4 py-2 font-mono text-neutral-400">{{ task.id }}</td>
              <td class="px-4 py-2 text-neutral-300">
                {{ task.kind }}
                <span
                  v-if="task.schedule_id !== null"
                  class="ml-1 rounded bg-neutral-800 px-1 py-0.5 font-mono text-[10px] text-neutral-500"
                >
                  定时任务 #{{ task.schedule_id }}
                </span>
              </td>
              <td class="px-4 py-2" :class="statusClass(task.status)">
                {{ statusLabel(task.status) }}
              </td>
              <td class="px-4 py-2 font-mono text-neutral-500">{{ task.attempts }}</td>
              <td class="px-4 py-2 font-mono text-xs text-neutral-600">
                {{ formatTime(task.created_at) }}
              </td>
              <td class="px-4 py-2 font-mono text-xs text-neutral-600">
                {{ formatTime(task.finished_at) }}
              </td>
            </tr>
            <tr v-if="tasks.length === 0">
              <td colspan="6" class="px-4 py-10 text-center text-sm text-neutral-600">
                还没有任务。填写番号提交一次刮削。
              </td>
            </tr>
          </tbody>
        </table>
      </section>

      <aside class="flex flex-col gap-4">
        <section v-if="activeTask" class="rounded-lg border border-neutral-800/80 p-4">
          <h2 class="text-sm font-medium text-neutral-200">任务 {{ activeTask.id }}</h2>
          <dl class="mt-3 space-y-1 text-xs">
            <div class="flex justify-between gap-3">
              <dt class="text-neutral-600">类型</dt>
              <dd class="text-neutral-300">{{ activeTask.kind }}</dd>
            </div>
            <div class="flex justify-between gap-3">
              <dt class="text-neutral-600">状态</dt>
              <dd :class="statusClass(activeTask.status)">{{ statusLabel(activeTask.status) }}</dd>
            </div>
            <div class="flex justify-between gap-3">
              <dt class="text-neutral-600">重试自</dt>
              <dd class="text-neutral-300">{{ activeTask.retry_of ?? "—" }}</dd>
            </div>
            <div v-if="activeTask.error" class="flex justify-between gap-3">
              <dt class="shrink-0 text-neutral-600">失败原因</dt>
              <dd class="text-right text-red-400">{{ activeTask.error }}</dd>
            </div>
          </dl>

          <div class="mt-4 flex gap-2">
            <Button
              v-if="!isTerminal(activeTask.status)"
              variant="ghost"
              @click="cancelTask(activeTask.id)"
            >
              取消
            </Button>
            <Button v-if="isRetryable(activeTask.status)" @click="retryTask(activeTask.id)">
              重试
            </Button>
          </div>

          <template v-if="children.length > 0">
            <h3 class="mt-5 text-xs font-medium text-neutral-400">子任务</h3>
            <ul class="mt-2 space-y-1 text-xs">
              <li v-for="child in children" :key="child.id" class="flex justify-between gap-2">
                <span class="text-neutral-400">#{{ child.id }} {{ child.kind }}</span>
                <span :class="statusClass(child.status)">{{ statusLabel(child.status) }}</span>
              </li>
            </ul>
          </template>

          <h3 class="mt-5 text-xs font-medium text-neutral-400">日志</h3>
          <ol class="mt-2 max-h-72 space-y-1 overflow-y-auto font-mono text-xs">
            <li
              v-for="(line, index) in logLines"
              :key="index"
              class="flex gap-2 text-neutral-400"
            >
              <span class="shrink-0 text-neutral-600">{{ formatTime(line.at).slice(11) }}</span>
              <span
                :class="
                  line.level === 'error'
                    ? 'text-red-400'
                    : line.level === 'warn'
                      ? 'text-amber-400'
                      : 'text-neutral-300'
                "
              >
                {{ line.message }}
              </span>
            </li>
            <li v-if="logLines.length === 0" class="text-neutral-600">暂无日志</li>
          </ol>
        </section>

        <section v-else class="rounded-lg border border-neutral-800/80 p-4 text-sm text-neutral-600">
          选择一条任务查看日志。
        </section>
      </aside>
    </main>
  </div>
</template>
