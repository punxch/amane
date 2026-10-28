<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, reactive, ref, watch } from "vue";

import {
  ApiError,
  api,
  type ExtractedNumber,
  type FilterOptions,
  type MetadataDto,
} from "@/api/client";
import HighlightedText from "@/components/HighlightedText.vue";
import Button from "@/components/ui/Button.vue";

const PAGE_SIZE = 50;

const items = ref<MetadataDto[]>([]);
const total = ref(0);
const pageIndex = ref(0);
const loading = ref(false);
const failure = ref<string | null>(null);
const notice = ref<{ kind: "error" | "ok"; text: string } | null>(null);

const filters = ref<FilterOptions | null>(null);
const filtersFailed = ref(false);
const query = reactive({ keyword: "", contentType: "", source: "" });

const form = reactive({ number: "", title: "" });
const probeText = ref("");
const probeResult = ref<ExtractedNumber | null>(null);
const detail = ref<MetadataDto | null>(null);

const pageCount = computed(() => Math.max(1, Math.ceil(total.value / PAGE_SIZE)));
const hasFilter = computed(
  () => query.keyword.trim() !== "" || query.contentType !== "" || query.source !== "",
);

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

let requestSeq = 0;

async function load(): Promise<void> {
  // 防抖只决定何时发请求; 慢查询下旧响应仍可能后到, 用它丢弃过期结果.
  const seq = ++requestSeq;
  loading.value = true;
  failure.value = null;
  try {
    const page = await api.list({
      keyword: query.keyword,
      contentType: query.contentType,
      source: query.source,
      limit: PAGE_SIZE,
      offset: pageIndex.value * PAGE_SIZE,
    });
    if (seq !== requestSeq) {
      return;
    }
    // 筛选后总页数可能变少: 停在越界页会看到空白, 回到最后一页.
    if (page.items.length === 0 && page.total > 0 && pageIndex.value > 0) {
      pageIndex.value = Math.max(0, Math.ceil(page.total / PAGE_SIZE) - 1);
      await load();
      return;
    }
    items.value = page.items;
    total.value = page.total;
  } catch (error) {
    if (seq !== requestSeq) {
      return;
    }
    // 保留总数: 否则页码会显示成「第 2 / 1 页」.
    items.value = [];
    failure.value = describe(error);
  } finally {
    if (seq === requestSeq) {
      loading.value = false;
    }
  }
}

function go(index: number): void {
  const target = Math.min(Math.max(index, 0), pageCount.value - 1);
  if (target === pageIndex.value) {
    return;
  }
  pageIndex.value = target;
  void load();
}

function resetFilters(): void {
  query.keyword = "";
  query.contentType = "";
  query.source = "";
}

function typeLabel(value: string): string {
  const option = filters.value?.content_types.find((entry) => entry.value === value);
  return option?.label ?? value;
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
    pageIndex.value = 0;
    await load();
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

let reloadTimer: ReturnType<typeof setTimeout> | undefined;

/**
 * 条件一变就回到第一页: 停在第 3 页会让筛选结果看起来是空的.
 *
 * 关键字逐字输入, 需要等待; 下拉框是离散选择, 立即生效. 两个入口共用同一个定时器,
 * 于是「清除筛选」这类同时改动多项的操作只会发一次请求.
 */
function scheduleReload(delay: number): void {
  if (reloadTimer !== undefined) {
    clearTimeout(reloadTimer);
  }
  reloadTimer = setTimeout(() => {
    pageIndex.value = 0;
    void load();
  }, delay);
}

watch(
  () => query.keyword,
  () => scheduleReload(250),
);

watch([() => query.contentType, () => query.source], () => scheduleReload(0));

onBeforeUnmount(() => {
  if (reloadTimer !== undefined) {
    clearTimeout(reloadTimer);
  }
});

onMounted(async () => {
  // 可选项只是下拉框内容, 取不到也不应阻塞列表本身.
  void api
    .filters()
    .then((value) => {
      filters.value = value;
      filtersFailed.value = false;
    })
    .catch(() => {
      filters.value = null;
      filtersFailed.value = true;
    });
  await load();
});
</script>

<template>
  <div>
    <div class="flex items-center justify-between px-8 pt-5">
      <span class="font-mono text-xs text-neutral-500">
        {{ loading ? "读取中" : `${total} 条` }}<span v-if="hasFilter"> · 已筛选</span>
      </span>
      <Button variant="ghost" :disabled="loading" @click="load">刷新</Button>
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
        <div class="flex flex-wrap items-center gap-2 border-b border-neutral-800/80 px-4 py-3">
          <input
            v-model="query.keyword"
            class="w-56 rounded-md border border-neutral-800 bg-neutral-900 px-3 py-1.5 text-sm text-neutral-100 outline-none focus:border-emerald-600"
            placeholder="搜索番号或标题"
          />
          <select
            v-model="query.contentType"
            class="scheme-dark rounded-md border border-neutral-800 bg-neutral-900 px-2 py-1.5 text-sm text-neutral-300 outline-none focus:border-emerald-600"
          >
            <option value="">全部类型</option>
            <option
              v-for="option in filters?.content_types ?? []"
              :key="option.value"
              :value="option.value"
            >
              {{ option.label }}
            </option>
          </select>
          <select
            v-model="query.source"
            class="scheme-dark rounded-md border border-neutral-800 bg-neutral-900 px-2 py-1.5 text-sm text-neutral-300 outline-none focus:border-emerald-600"
          >
            <option value="">全部来源</option>
            <option v-for="option in filters?.sources ?? []" :key="option.id" :value="option.id">
              {{ option.label }}
            </option>
          </select>
          <div class="ml-auto flex items-center gap-3">
            <span v-if="filtersFailed" class="text-xs text-amber-600">
              筛选项未加载，只能按关键字检索
            </span>
            <button
              v-if="hasFilter"
              class="text-xs text-neutral-500 hover:text-neutral-300"
              @click="resetFilters"
            >
              清除筛选
            </button>
          </div>
        </div>

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
            <tr v-if="loading && items.length === 0">
              <td colspan="4" class="px-4 py-10 text-center text-sm text-neutral-600">读取中…</td>
            </tr>

            <tr v-else-if="failure">
              <td colspan="4" class="px-4 py-10 text-center">
                <p class="text-sm text-red-400">{{ failure }}</p>
                <Button class="mt-3" variant="ghost" @click="load">重试</Button>
              </td>
            </tr>

            <tr v-else-if="items.length === 0">
              <td colspan="4" class="px-4 py-10 text-center text-sm text-neutral-600">
                <template v-if="hasFilter">
                  没有符合条件的影片。
                  <button class="text-emerald-500 hover:text-emerald-400" @click="resetFilters">
                    清除筛选
                  </button>
                </template>
                <template v-else>尚无影片。填写番号写入一条，或先运行一次刮削。</template>
              </td>
            </tr>

            <tr
              v-for="item in items"
              :key="item.id"
              class="cursor-pointer border-b border-neutral-900 last:border-0 hover:bg-neutral-900/40"
              @click="inspect(item)"
            >
              <td class="px-4 py-2 font-mono text-neutral-200">
                <HighlightedText :text="item.number" :keyword="query.keyword" />
              </td>
              <td class="px-4 py-2 text-neutral-400">
                <HighlightedText v-if="item.title" :text="item.title" :keyword="query.keyword" />
                <template v-else>—</template>
              </td>
              <td class="px-4 py-2 text-neutral-500">{{ typeLabel(item.content_type) }}</td>
              <td class="px-4 py-2 font-mono text-xs text-neutral-600">
                {{ formatTime(item.updated_at) }}
              </td>
            </tr>
          </tbody>
        </table>

        <div
          class="flex items-center justify-between border-t border-neutral-800/80 px-4 py-2 text-xs text-neutral-500"
        >
          <span>第 {{ pageIndex + 1 }} / {{ pageCount }} 页</span>
          <div class="flex gap-2">
            <button
              class="rounded border border-neutral-800 px-2 py-0.5 disabled:opacity-40"
              :disabled="loading || pageIndex === 0"
              @click="go(pageIndex - 1)"
            >
              上一页
            </button>
            <button
              class="rounded border border-neutral-800 px-2 py-0.5 disabled:opacity-40"
              :disabled="loading || pageIndex + 1 >= pageCount"
              @click="go(pageIndex + 1)"
            >
              下一页
            </button>
          </div>
        </div>
      </section>

      <aside class="flex flex-col gap-4">
        <section v-if="detail" class="rounded-lg border border-neutral-800/80 p-4">
          <div class="flex items-start justify-between gap-2">
            <h2 class="font-mono text-sm text-neutral-200">{{ detail.number }}</h2>
            <button class="text-xs text-neutral-600 hover:text-neutral-400" @click="detail = null">
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

          <div v-if="detail.sources.length > 0" class="mt-3 border-t border-neutral-800/60 pt-2">
            <p class="text-xs text-neutral-600">来源: {{ detail.sources.join(" / ") }}</p>
            <dl class="mt-1 space-y-0.5 font-mono text-[11px] text-neutral-500">
              <div
                v-for="(source, field) in detail.field_sources"
                :key="field"
                class="flex justify-between gap-3"
              >
                <dt>{{ field }}</dt>
                <dd>{{ source }}</dd>
              </div>
            </dl>
          </div>
        </section>

        <form class="rounded-lg border border-neutral-800/80 p-4" @submit.prevent="submit">
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
              <dd>{{ typeLabel(probeResult.content_type) }}</dd>
            </div>
          </dl>
        </section>
      </aside>
    </main>
  </div>
</template>
