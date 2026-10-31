<script setup lang="ts">
import { computed, onMounted, ref } from "vue";

import { api, describeError, type ScheduleDto } from "@/api/client";
import EmptyState from "@/components/common/EmptyState.vue";
import PageShell from "@/components/common/PageShell.vue";
import Alert from "@/components/ui/Alert.vue";
import Badge from "@/components/ui/Badge.vue";
import Button from "@/components/ui/Button.vue";
import Modal from "@/components/ui/Modal.vue";
import Paper from "@/components/ui/Paper.vue";
import Switch from "@/components/ui/Switch.vue";
import TextInput from "@/components/ui/TextInput.vue";
import { formatTime } from "@/lib/format";
import { taskKind } from "@/lib/task/status";

const DEFAULT_CRON = "0 3 * * *";
const DEFAULT_TIMEZONE = "Asia/Shanghai";

const schedules = ref<ScheduleDto[]>([]);
const loading = ref(false);
const failure = ref<string | null>(null);
const notice = ref<{ kind: "error" | "ok"; text: string } | null>(null);

const createOpen = ref(false);
const form = ref({ number: "", cron: DEFAULT_CRON, timezone: DEFAULT_TIMEZONE });
const creating = ref(false);

const removing = ref<ScheduleDto | null>(null);
const removeBusy = ref(false);
const toggling = ref<number | null>(null);

const count = computed(() => schedules.value.length);

function report(kind: "error" | "ok", text: string): void {
  notice.value = { kind, text };
}

async function refresh(): Promise<void> {
  loading.value = true;
  failure.value = null;
  try {
    schedules.value = (await api.listSchedules()).items;
  } catch (error) {
    failure.value = describeError(error);
  } finally {
    loading.value = false;
  }
}

async function create(): Promise<void> {
  const number = form.value.number.trim();
  if (number === "") {
    report("error", "请填写番号");
    return;
  }
  creating.value = true;
  try {
    await api.createSchedule({
      kind: "scrape",
      payload: { number },
      cron: form.value.cron.trim(),
      timezone: form.value.timezone.trim(),
    });
    createOpen.value = false;
    form.value = { number: "", cron: DEFAULT_CRON, timezone: DEFAULT_TIMEZONE };
    report("ok", "定时任务已登记");
    await refresh();
  } catch (error) {
    report("error", describeError(error));
  } finally {
    creating.value = false;
  }
}

async function toggle(schedule: ScheduleDto, enabled: boolean): Promise<void> {
  toggling.value = schedule.id;
  try {
    await api.updateSchedule(schedule.id, { enabled });
    await refresh();
  } catch (error) {
    report("error", describeError(error));
  } finally {
    toggling.value = null;
  }
}

async function remove(): Promise<void> {
  const target = removing.value;
  if (target === null) {
    return;
  }
  removeBusy.value = true;
  try {
    await api.deleteSchedule(target.id);
    removing.value = null;
    report("ok", `已删除定时任务 #${target.id}`);
    await refresh();
  } catch (error) {
    report("error", describeError(error));
  } finally {
    removeBusy.value = false;
  }
}

/** 载荷的番号. 断言必要性: 载荷结构由任务类型决定, 生成的类型是开放的 JSON 对象. */
function payloadNumber(schedule: ScheduleDto): string {
  const payload = schedule.payload as { number?: string };
  return payload.number ?? "";
}

onMounted(refresh);
</script>

<template>
  <PageShell title="定时任务" :subtitle="loading ? '读取中' : `${count} 条`">
    <template #actions>
      <Button variant="default" :disabled="loading" @click="refresh">刷新</Button>
      <Button @click="createOpen = true">新建定时任务</Button>
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
      v-else-if="!loading && schedules.length === 0"
      icon="clock"
      title="还没有定时任务"
      description="登记一条 cron 表达式，按所选时区触发。"
    >
      <Button size="sm" @click="createOpen = true">新建定时任务</Button>
    </EmptyState>

    <Paper v-else padding="none">
      <table class="amane-table">
        <thead>
          <tr>
            <th style="width: 64px">编号</th>
            <th>内容</th>
            <th style="width: 150px">表达式</th>
            <th style="width: 160px">时区</th>
            <th style="width: 170px">下次触发</th>
            <th style="width: 80px">启用</th>
            <th style="width: 100px"></th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="schedule in schedules" :key="schedule.id">
            <td class="mono dimmed">{{ schedule.id }}</td>
            <td>
              <span class="mono">{{ taskKind(schedule.kind) }}</span>
              <Badge v-if="payloadNumber(schedule) !== ''" tone="gray" size="sm" class="payload">
                {{ payloadNumber(schedule) }}
              </Badge>
            </td>
            <td class="mono">{{ schedule.cron }}</td>
            <td class="mono dimmed small">{{ schedule.timezone }}</td>
            <td class="mono small">{{ schedule.enabled ? formatTime(schedule.next_fire_at) : "—" }}</td>
            <td>
              <Switch
                :model-value="schedule.enabled"
                :label="schedule.enabled ? '停用定时任务' : '启用定时任务'"
                :disabled="toggling === schedule.id"
                @update:model-value="(value) => toggle(schedule, value)"
              />
            </td>
            <td class="cell-action">
              <Button variant="subtle" size="sm" @click="removing = schedule">删除</Button>
            </td>
          </tr>
        </tbody>
      </table>
    </Paper>

    <Modal :opened="createOpen" title="新建定时任务" :width="520" @close="createOpen = false">
      <p class="modal-hint">
        触发时刻按所选时区计算；停机期间错过的触发不逐个重放。表达式是 5 段 cron。
      </p>
      <label class="field">
        <span>番号</span>
        <TextInput v-model="form.number" mono placeholder="SSIS-001" />
      </label>
      <label class="field">
        <span>cron 表达式</span>
        <TextInput v-model="form.cron" mono placeholder="0 3 * * *" />
      </label>
      <label class="field">
        <span>时区（IANA 名称）</span>
        <TextInput v-model="form.timezone" mono placeholder="Asia/Shanghai" />
      </label>
      <template #footer>
        <Button variant="default" @click="createOpen = false">取消</Button>
        <Button :loading="creating" @click="create">登记</Button>
      </template>
    </Modal>

    <Modal :opened="removing !== null" title="删除定时任务" :width="420" @close="removing = null">
      <p class="modal-hint">
        删除定时任务 <strong>#{{ removing?.id }}</strong>；已派生的任务不受影响。
      </p>
      <template #footer>
        <Button variant="default" @click="removing = null">取消</Button>
        <Button variant="danger" :loading="removeBusy" @click="remove">删除</Button>
      </template>
    </Modal>
  </PageShell>
</template>

<style scoped>
.mono {
  font-family: var(--font-mono);
  font-size: 13px;
}

.dimmed {
  color: var(--amane-dimmed);
}

.small {
  font-size: 12px;
}

.payload {
  margin-left: 6px;
}

.cell-action {
  text-align: right;
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
