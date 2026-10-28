<script setup lang="ts">
import { onMounted, reactive, ref } from "vue";

import {
  ApiError,
  api,
  type ScheduleDto,
} from "@/api/client";
import Button from "@/components/ui/Button.vue";

const schedules = ref<ScheduleDto[]>([]);
const loading = ref(false);
const failure = ref<string | null>(null);
const notice = ref<{ kind: "error" | "ok"; text: string } | null>(null);

const form = reactive({ number: "", cron: "0 3 * * *", timezone: "Asia/Shanghai" });

function report(kind: "error" | "ok", text: string): void {
  notice.value = { kind, text };
}

function describe(error: unknown): string {
  if (error instanceof ApiError) {
    return error.code === "unreachable"
      ? "无法连接后端服务，请确认服务已启动后重试。"
      : error.message;
  }
  return "请求失败，请稍后重试。";
}

async function refresh(): Promise<void> {
  loading.value = true;
  failure.value = null;
  try {
    schedules.value = (await api.listSchedules()).items;
  } catch (error) {
    failure.value = describe(error);
  } finally {
    loading.value = false;
  }
}

async function create(): Promise<void> {
  if (form.number.trim() === "") {
    report("error", "请填写番号");
    return;
  }
  try {
    await api.createSchedule({
      kind: "scrape",
      payload: { number: form.number.trim() },
      cron: form.cron.trim(),
      timezone: form.timezone.trim(),
    });
    report("ok", "定时任务已登记");
    form.number = "";
    await refresh();
  } catch (error) {
    report("error", describe(error));
  }
}

async function toggle(schedule: ScheduleDto): Promise<void> {
  try {
    await api.updateSchedule(schedule.id, { enabled: !schedule.enabled });
    report("ok", schedule.enabled ? "已停用" : "已启用");
    await refresh();
  } catch (error) {
    report("error", describe(error));
  }
}

async function remove(schedule: ScheduleDto): Promise<void> {
  try {
    await api.deleteSchedule(schedule.id);
    report("ok", `已删除定时任务 #${schedule.id}`);
    await refresh();
  } catch (error) {
    report("error", describe(error));
  }
}

/** 载荷的番号. 断言必要性: 载荷结构由任务类型决定, 生成的类型是开放的 JSON 对象. */
function payloadNumber(schedule: ScheduleDto): string {
  const payload = schedule.payload as { number?: string };
  return payload?.number ?? "";
}

function formatTime(value: string | null | undefined): string {
  return value === null || value === undefined
    ? "—"
    : new Date(value).toLocaleString("zh-CN", { hour12: false });
}

onMounted(refresh);
</script>

<template>
  <div>
    <div class="flex items-center justify-between px-8 pt-5">
      <span class="font-mono text-xs text-neutral-500">
        {{ loading ? "读取中" : `${schedules.length} 条` }}
      </span>
      <Button variant="ghost" :disabled="loading" @click="refresh">刷新</Button>
    </div>

    <p
      v-if="notice"
      :class="[
        'mx-8 mt-4 rounded-md border px-4 py-2 text-sm',
        notice.kind === 'error'
          ? 'border-red-900/60 bg-red-950/40 text-red-300'
          : 'border-emerald-900/60 bg-emerald-950/40 text-emerald-300',
      ]"
    >
      {{ notice.text }}
    </p>

    <main class="grid items-start gap-6 px-8 py-6 lg:grid-cols-[minmax(0,1fr)_320px]">
      <section class="overflow-hidden rounded-lg border border-neutral-800/80">
        <table class="w-full border-collapse text-sm">
          <thead>
            <tr class="border-b border-neutral-800/80 text-left text-xs text-neutral-500">
              <th class="px-4 py-2 font-normal">编号</th>
              <th class="px-4 py-2 font-normal">内容</th>
              <th class="px-4 py-2 font-normal">表达式</th>
              <th class="px-4 py-2 font-normal">时区</th>
              <th class="px-4 py-2 font-normal">下次触发</th>
              <th class="px-4 py-2 font-normal">状态</th>
              <th class="px-4 py-2 font-normal"></th>
            </tr>
          </thead>
          <tbody>
            <tr v-if="failure">
              <td colspan="7" class="px-4 py-10 text-center">
                <p class="text-sm text-red-400">{{ failure }}</p>
                <Button class="mt-3" variant="ghost" @click="refresh">重试</Button>
              </td>
            </tr>

            <tr v-else-if="schedules.length === 0">
              <td colspan="7" class="px-4 py-10 text-center text-sm text-neutral-600">
                还没有定时任务。在右侧登记一条 cron 表达式。
              </td>
            </tr>

            <tr
              v-for="schedule in schedules"
              :key="schedule.id"
              class="border-b border-neutral-900 last:border-0"
            >
              <td class="px-4 py-2 font-mono text-neutral-500">{{ schedule.id }}</td>
              <td class="px-4 py-2 font-mono text-neutral-300">
                {{ schedule.kind }} {{ payloadNumber(schedule) }}
              </td>
              <td class="px-4 py-2 font-mono text-neutral-300">{{ schedule.cron }}</td>
              <td class="px-4 py-2 font-mono text-xs text-neutral-500">{{ schedule.timezone }}</td>
              <td class="px-4 py-2 font-mono text-xs text-neutral-400">
                {{ schedule.enabled ? formatTime(schedule.next_fire_at) : "—" }}
              </td>
              <td class="px-4 py-2">
                <span
                  :class="
                    schedule.enabled ? 'text-emerald-400' : 'text-neutral-600'
                  "
                >
                  {{ schedule.enabled ? "启用" : "停用" }}
                </span>
              </td>
              <td class="px-4 py-2 text-right text-xs">
                <button
                  class="mr-3 text-neutral-500 hover:text-neutral-300"
                  @click="toggle(schedule)"
                >
                  {{ schedule.enabled ? "停用" : "启用" }}
                </button>
                <button class="text-neutral-500 hover:text-red-400" @click="remove(schedule)">
                  删除
                </button>
              </td>
            </tr>
          </tbody>
        </table>
      </section>

      <aside>
        <form class="rounded-lg border border-neutral-800/80 p-4" @submit.prevent="create">
          <h2 class="text-sm font-medium text-neutral-200">登记定时任务</h2>
          <p class="mt-1 text-xs text-neutral-500">
            触发时刻按所选时区计算；停机期间错过的触发不逐个重放。
          </p>
          <label class="mt-3 block text-xs text-neutral-500">
            番号
            <input
              v-model="form.number"
              class="mt-1 w-full rounded-md border border-neutral-800 bg-neutral-900 px-3 py-1.5 font-mono text-sm text-neutral-100 outline-none focus:border-emerald-600"
              placeholder="SSIS-001"
            />
          </label>
          <label class="mt-3 block text-xs text-neutral-500">
            cron 表达式
            <input
              v-model="form.cron"
              class="mt-1 w-full rounded-md border border-neutral-800 bg-neutral-900 px-3 py-1.5 font-mono text-sm text-neutral-100 outline-none focus:border-emerald-600"
              placeholder="0 3 * * *"
            />
          </label>
          <label class="mt-3 block text-xs text-neutral-500">
            时区（IANA 名称）
            <input
              v-model="form.timezone"
              class="mt-1 w-full rounded-md border border-neutral-800 bg-neutral-900 px-3 py-1.5 font-mono text-sm text-neutral-100 outline-none focus:border-emerald-600"
              placeholder="Asia/Shanghai"
            />
          </label>
          <Button class="mt-4 w-full" type="submit">登记</Button>
        </form>
      </aside>
    </main>
  </div>
</template>
