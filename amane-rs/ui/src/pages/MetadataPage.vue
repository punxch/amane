<script setup lang="ts">
import { onMounted, reactive, ref } from "vue";

import { ApiError, api, type ExtractedNumber, type MetadataDto } from "@/api/client";
import Button from "@/components/ui/Button.vue";

const items = ref<MetadataDto[]>([]);
const total = ref(0);
const loading = ref(false);
const notice = ref<{ kind: "error" | "ok"; text: string } | null>(null);

const form = reactive({ number: "", title: "" });
const probeText = ref("");
const probeResult = ref<ExtractedNumber | null>(null);
const detail = ref<MetadataDto | null>(null);

function report(kind: "error" | "ok", text: string): void {
  notice.value = { kind, text };
}

function describe(error: unknown): string {
  return error instanceof ApiError ? error.message : "请求失败";
}

async function refresh(): Promise<void> {
  loading.value = true;
  try {
    const page = await api.list();
    items.value = page.items;
    total.value = page.total;
  } catch (error) {
    report("error", describe(error));
  } finally {
    loading.value = false;
  }
}

async function inspect(item: MetadataDto): Promise<void> {
  try {
    detail.value = await api.metadata(item.number);
  } catch (error) {
    report("error", describe(error));
  }
}

async function submit(): Promise<void> {
  if (form.number.trim() === "") {
    report("error", "请填写番号");
    return;
  }
  try {
    const created = await api.create({
      number: form.number.trim(),
      title: form.title.trim() || null,
    });
    report("ok", `已写入 ${created.number}`);
    form.number = "";
    form.title = "";
    await refresh();
  } catch (error) {
    report("error", describe(error));
  }
}

async function detect(): Promise<void> {
  if (probeText.value.trim() === "") {
    report("error", "请填写待解析的文本");
    return;
  }
  try {
    probeResult.value = await api.extract(probeText.value.trim());
  } catch (error) {
    probeResult.value = null;
    report("error", describe(error));
  }
}

function formatTime(value: string): string {
  return new Date(value).toLocaleString("zh-CN", { hour12: false });
}

onMounted(refresh);
</script>

<template>
  <div>
    <div class="flex items-center justify-between px-8 pt-5">
      <span class="font-mono text-xs text-neutral-500">{{ total }} 条</span>
      <Button variant="ghost" :disabled="loading" @click="refresh">
        {{ loading ? "读取中" : "刷新" }}
      </Button>
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

    <main class="grid gap-6 px-8 py-6 lg:grid-cols-[minmax(0,1fr)_320px]">
      <section class="overflow-hidden rounded-lg border border-neutral-800/80">
        <table class="w-full border-collapse text-sm">
          <thead>
            <tr class="border-b border-neutral-800/80 text-left text-xs text-neutral-500">
              <th class="px-4 py-2 font-normal">番号</th>
              <th class="px-4 py-2 font-normal">标题</th>
              <th class="px-4 py-2 font-normal">类型</th>
              <th class="px-4 py-2 font-normal">更新时间</th>
            </tr>
          </thead>
          <tbody>
            <tr
              v-for="item in items"
              :key="item.id"
              class="cursor-pointer border-b border-neutral-900 last:border-0 hover:bg-neutral-900/40"
              @click="inspect(item)"
            >
              <td class="px-4 py-2 font-mono text-neutral-200">{{ item.number }}</td>
              <td class="px-4 py-2 text-neutral-400">{{ item.title ?? "—" }}</td>
              <td class="px-4 py-2 text-neutral-500">{{ item.content_type }}</td>
              <td class="px-4 py-2 font-mono text-xs text-neutral-600">
                {{ formatTime(item.updated_at) }}
              </td>
            </tr>
            <tr v-if="items.length === 0">
              <td colspan="4" class="px-4 py-10 text-center text-sm text-neutral-600">
                尚无影片。填写番号写入一条，或先运行一次刮削。
              </td>
            </tr>
          </tbody>
        </table>
      </section>

      <aside class="flex flex-col gap-4">
        <section
          v-if="detail"
          class="rounded-lg border border-neutral-800/80 p-4"
        >
          <div class="flex items-start justify-between gap-2">
            <h2 class="font-mono text-sm text-neutral-200">{{ detail.number }}</h2>
            <button
              class="text-xs text-neutral-600 hover:text-neutral-400"
              @click="detail = null"
            >
              关闭
            </button>
          </div>
          <img
            v-if="detail.poster_local_url"
            :src="detail.poster_local_url"
            alt=""
            class="mt-3 w-full rounded-md border border-neutral-800 object-cover"
          />
          <p class="mt-2 text-sm leading-snug text-neutral-300">{{ detail.title ?? "无标题" }}</p>
          <dl class="mt-3 space-y-1 text-xs">
            <div class="flex justify-between gap-3">
              <dt class="text-neutral-600">上映日期</dt>
              <dd class="text-neutral-300">{{ detail.release_date ?? "—" }}</dd>
            </div>
            <div class="flex justify-between gap-3">
              <dt class="text-neutral-600">时长</dt>
              <dd class="text-neutral-300">
                {{ detail.runtime_minutes === null ? "—" : `${detail.runtime_minutes} 分钟` }}
              </dd>
            </div>
            <div class="flex justify-between gap-3">
              <dt class="text-neutral-600">制作商</dt>
              <dd class="text-right text-neutral-300">{{ detail.maker ?? "—" }}</dd>
            </div>
            <div class="flex justify-between gap-3">
              <dt class="text-neutral-600">厂牌</dt>
              <dd class="text-right text-neutral-300">{{ detail.label ?? "—" }}</dd>
            </div>
            <div class="flex justify-between gap-3">
              <dt class="shrink-0 text-neutral-600">演员</dt>
              <dd class="text-right text-neutral-300">
                {{ detail.actors.length === 0 ? "—" : detail.actors.join("、") }}
              </dd>
            </div>
          </dl>
          <p
            v-if="detail.plot"
            class="mt-3 max-h-40 overflow-y-auto border-t border-neutral-800/60 pt-2 text-xs leading-relaxed text-neutral-500"
          >
            {{ detail.plot }}
          </p>
          <p
            v-if="detail.poster_url && !detail.poster_local_url"
            class="mt-2 truncate font-mono text-xs text-amber-600"
          >
            海报尚未存到本地: {{ detail.poster_url }}
          </p>
        </section>

        <form
          class="rounded-lg border border-neutral-800/80 p-4"
          @submit.prevent="submit"
        >
          <h2 class="text-sm font-medium text-neutral-200">写入影片</h2>
          <label class="mt-3 block text-xs text-neutral-500">
            番号
            <input
              v-model="form.number"
              class="mt-1 w-full rounded-md border border-neutral-800 bg-neutral-900 px-3 py-1.5 font-mono text-sm text-neutral-100 outline-none focus:border-emerald-600"
              placeholder="SSIS001"
            />
          </label>
          <label class="mt-3 block text-xs text-neutral-500">
            标题
            <input
              v-model="form.title"
              class="mt-1 w-full rounded-md border border-neutral-800 bg-neutral-900 px-3 py-1.5 text-sm text-neutral-100 outline-none focus:border-emerald-600"
              placeholder="可留空"
            />
          </label>
          <Button class="mt-4 w-full" type="submit">写入</Button>
        </form>

        <section class="rounded-lg border border-neutral-800/80 p-4">
          <h2 class="text-sm font-medium text-neutral-200">解析番号</h2>
          <p class="mt-1 text-xs text-neutral-500">粘贴文件名或完整路径。</p>
          <input
            v-model="probeText"
            class="mt-3 w-full rounded-md border border-neutral-800 bg-neutral-900 px-3 py-1.5 font-mono text-sm text-neutral-100 outline-none focus:border-emerald-600"
            placeholder="[javdb]ABC-123-C.mp4"
          />
          <Button class="mt-3 w-full" variant="ghost" @click="detect">解析</Button>
          <dl v-if="probeResult" class="mt-3 space-y-1 font-mono text-xs text-neutral-400">
            <div class="flex justify-between">
              <dt class="text-neutral-600">番号</dt>
              <dd>{{ probeResult.number }}</dd>
            </div>
            <div class="flex justify-between">
              <dt class="text-neutral-600">前缀</dt>
              <dd>{{ probeResult.prefix || "—" }}</dd>
            </div>
            <div class="flex justify-between">
              <dt class="text-neutral-600">类型</dt>
              <dd>{{ probeResult.content_type }}</dd>
            </div>
          </dl>
        </section>
      </aside>
    </main>
  </div>
</template>
