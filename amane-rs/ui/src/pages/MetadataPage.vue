<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, reactive, ref, watch } from "vue";

import {
  describeError,
  api,
  type ExtractedNumber,
  type FilterOptions,
  type MetadataDto,
  type MetadataFieldDto,
  type UpdateMetadataRequest,
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
const query = reactive({ keyword: "", contentType: "", source: "", genre: "" });

const form = reactive({ number: "", title: "" });
const probeText = ref("");
const probeResult = ref<ExtractedNumber | null>(null);
const detail = ref<MetadataDto | null>(null);
const editing = ref<{ field: string; value: string } | null>(null);
const noteDraft = ref("");
const tagDraft = ref("");

const pageCount = computed(() => Math.max(1, Math.ceil(total.value / PAGE_SIZE)));
const hasFilter = computed(
  () =>
    query.keyword.trim() !== "" ||
    query.contentType !== "" ||
    query.source !== "" ||
    query.genre !== "",
);

function report(kind: "error" | "ok", text: string): void {
  notice.value = { kind, text };
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
      genre: query.genre,
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
    failure.value = describeError(error);
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

function showDetail(dto: MetadataDto): void {
  detail.value = dto;
  noteDraft.value = dto.note ?? "";
  editing.value = null;
}

async function inspect(item: MetadataDto): Promise<void> {
  try {
    showDetail(await api.metadata(item.number));
  } catch (error) {
    report("error", describeError(error));
  }
}

/** 字段来源的展示名称; `manual` 是人工修订的固定标识. */
function sourceLabel(source: string | null | undefined): string {
  if (source === null || source === undefined) {
    return "未标注";
  }
  return source === "manual" ? "人工" : source;
}

function startEdit(field: MetadataFieldDto): void {
  editing.value = { field: field.field, value: field.value ?? "" };
}

function cancelEdit(): void {
  editing.value = null;
}

async function saveEdit(): Promise<void> {
  if (editing.value === null || detail.value === null) {
    return;
  }
  const { field, value } = editing.value;

  // 断言必要性: 字段名在运行时才确定 (来自服务端的逐字段视图), TypeScript 无法把
  // 动态键收窄到具体属性; 收窄点集中在这一处, 键与 PATCH 的字段一一对应.
  const payload = (
    field === "runtime_minutes"
      ? { runtime_minutes: Number.parseInt(value.trim(), 10) }
      : { [field]: value }
  ) as UpdateMetadataRequest;
  if (field === "runtime_minutes" && Number.isNaN(payload.runtime_minutes)) {
    report("error", "时长应为整数分钟");
    return;
  }

  try {
    showDetail(await api.updateMetadata(detail.value.number, payload));
    report("ok", "已保存，该字段已锁定");
    void load();
  } catch (error) {
    report("error", describeError(error));
  }
}

async function toggleLock(field: MetadataFieldDto): Promise<void> {
  if (detail.value === null) {
    return;
  }
  try {
    showDetail(await api.setLock(detail.value.number, field.field, !field.locked));
    report("ok", field.locked ? `已解锁${field.label}` : `已锁定${field.label}`);
  } catch (error) {
    report("error", describeError(error));
  }
}

async function saveNote(): Promise<void> {
  if (detail.value === null) {
    return;
  }
  try {
    showDetail(await api.updateMetadata(detail.value.number, { note: noteDraft.value }));
    report("ok", "备注已保存");
  } catch (error) {
    report("error", describeError(error));
  }
}

async function addTag(): Promise<void> {
  const name = tagDraft.value.trim();
  if (detail.value === null || name === "") {
    return;
  }
  try {
    showDetail(await api.addTag(detail.value.number, name));
    tagDraft.value = "";
  } catch (error) {
    report("error", describeError(error));
  }
}

async function removeTag(name: string): Promise<void> {
  if (detail.value === null) {
    return;
  }
  try {
    showDetail(await api.removeTag(detail.value.number, name));
  } catch (error) {
    report("error", describeError(error));
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
    report("error", describeError(error));
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
    report("error", describeError(error));
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
          <select
            v-model="query.genre"
            class="scheme-dark rounded-md border border-neutral-800 bg-neutral-900 px-2 py-1.5 text-sm text-neutral-300 outline-none focus:border-emerald-600"
          >
            <option value="">全部分类</option>
            <option v-for="genre in filters?.genres ?? []" :key="genre" :value="genre">
              {{ genre }}
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

          <div class="mt-3 space-y-1.5">
            <div
              v-for="field in detail.fields"
              :key="field.field"
              class="rounded-md border border-neutral-900 px-2 py-1.5"
            >
              <div class="flex items-center justify-between gap-2">
                <span class="text-[11px] text-neutral-600">{{ field.label }}</span>
                <div class="flex items-center gap-2 text-[11px]">
                  <span
                    v-if="field.value !== null"
                    :class="field.source === 'manual' ? 'text-amber-500' : 'text-neutral-600'"
                  >
                    {{ sourceLabel(field.source) }}
                  </span>
                  <button
                    :class="
                      field.locked
                        ? 'text-amber-500 hover:text-amber-400'
                        : 'text-neutral-600 hover:text-neutral-300'
                    "
                    @click="toggleLock(field)"
                  >
                    {{ field.locked ? "解锁" : "锁定" }}
                  </button>
                  <button
                    v-if="field.field !== 'poster_url'"
                    class="text-neutral-600 hover:text-neutral-300"
                    @click="startEdit(field)"
                  >
                    修改
                  </button>
                </div>
              </div>

              <div v-if="editing?.field === field.field" class="mt-1">
                <textarea
                  v-if="field.field === 'plot'"
                  v-model="editing.value"
                  rows="3"
                  class="w-full rounded border border-neutral-800 bg-neutral-900 px-2 py-1 text-xs text-neutral-100 outline-none focus:border-emerald-600"
                />
                <input
                  v-else
                  v-model="editing.value"
                  class="w-full rounded border border-neutral-800 bg-neutral-900 px-2 py-1 text-xs text-neutral-100 outline-none focus:border-emerald-600"
                />
                <div class="mt-1 flex justify-end gap-2 text-[11px]">
                  <button class="text-neutral-500 hover:text-neutral-300" @click="cancelEdit">
                    取消
                  </button>
                  <button class="text-emerald-500 hover:text-emerald-400" @click="saveEdit">
                    保存
                  </button>
                </div>
              </div>
              <p
                v-else
                class="mt-0.5 whitespace-pre-wrap break-words text-xs leading-relaxed text-neutral-300"
              >
                {{ field.value ?? "—" }}
              </p>
            </div>
          </div>

          <p
            v-if="detail.poster_url && !detail.poster_local_url"
            class="mt-2 truncate font-mono text-xs text-amber-600"
          >
            海报尚未存到本地: {{ detail.poster_url }}
          </p>

          <p v-if="detail.genres.length > 0" class="mt-2 text-xs text-neutral-400">
            分类: {{ detail.genres.join("、") }}
          </p>

          <p v-if="detail.actors.length > 0" class="mt-2 text-xs text-neutral-400">
            演员: {{ detail.actors.join("、") }}
          </p>

          <div class="mt-3 border-t border-neutral-800/60 pt-2">
            <span class="text-[11px] text-neutral-600">标签</span>
            <div class="mt-1 flex flex-wrap items-center gap-1">
              <span
                v-for="tag in detail.tags"
                :key="tag"
                class="inline-flex items-center gap-1 rounded bg-neutral-800 px-1.5 py-0.5 text-[11px] text-neutral-300"
              >
                {{ tag }}
                <button class="text-neutral-500 hover:text-red-400" @click="removeTag(tag)">
                  ×
                </button>
              </span>
              <input
                v-model="tagDraft"
                class="w-24 rounded border border-neutral-800 bg-neutral-900 px-1.5 py-0.5 text-[11px] text-neutral-100 outline-none focus:border-emerald-600"
                placeholder="新标签"
                @keyup.enter="addTag"
              />
              <button class="text-[11px] text-neutral-500 hover:text-neutral-300" @click="addTag">
                添加
              </button>
            </div>
          </div>

          <div class="mt-3 border-t border-neutral-800/60 pt-2">
            <div class="flex items-center justify-between">
              <span class="text-[11px] text-neutral-600">备注</span>
              <button
                class="text-[11px] text-neutral-500 hover:text-neutral-300"
                @click="saveNote"
              >
                保存
              </button>
            </div>
            <textarea
              v-model="noteDraft"
              rows="2"
              class="mt-1 w-full rounded border border-neutral-800 bg-neutral-900 px-2 py-1 text-xs text-neutral-100 outline-none focus:border-emerald-600"
              placeholder="写给自己的备注"
            />
          </div>

          <p v-if="detail.sources.length > 0" class="mt-3 text-xs text-neutral-600">
            本次命中来源: {{ detail.sources.join(" / ") }}
          </p>
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
