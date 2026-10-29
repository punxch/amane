<script setup lang="ts">
import { onMounted, reactive, ref } from "vue";

import {
  describeError,
  api,
  type CleanupManifestDto,
  type LibraryDto,
  type LibraryFilePage,
  type OrganizePreviewDto,
  type TrashPageDto,
} from "@/api/client";
import Button from "@/components/ui/Button.vue";

/** 呈现顺序固定: 参与处理的类别在前, 未识别兜底, 防止任何一类被静默忽略. */
const KINDS: Array<{ value: string; label: string }> = [
  { value: "media", label: "媒体文件" },
  { value: "unwanted", label: "无效文件" },
  { value: "sidecar", label: "附属文件" },
  { value: "system", label: "系统产物" },
  { value: "leftover_dir", label: "残留目录" },
  { value: "unrecognized", label: "未识别" },
];

/** 冲突类别的展示文案. */
const CONFLICT_LABELS: Record<string, string> = {
  exists: "目标已存在",
  number_mismatch: "同名不同番号",
  same_path: "源与目标相同",
};

const libraries = ref<LibraryDto[]>([]);
const loading = ref(false);
const scanning = ref<number | null>(null);
const failure = ref<string | null>(null);
const notice = ref<{ kind: "error" | "ok"; text: string } | null>(null);
const form = reactive({ path: "" });

/** 当前展开的媒体库与其文件列表. */
const expanded = ref<number | null>(null);
const expandedKind = ref<string | null>(null);
const expandedFiles = ref<LibraryFilePage | null>(null);
const filesLoading = ref(false);

/** 整理预览. 只读: 后端不建目录不移动文件, 重复预览结果一致. */
const preview = ref<OrganizePreviewDto | null>(null);
const previewLibrary = ref<number | null>(null);
const previewLoading = ref(false);
const executing = ref(false);

/** 清理清单与回收目录. 清单为空时执行入口不可用. */
const cleanup = ref<CleanupManifestDto | null>(null);
const cleanupLibrary = ref<number | null>(null);
const cleanupLoading = ref(false);
const cleaningUp = ref(false);
const trash = ref<TrashPageDto | null>(null);
const trashLibrary = ref<number | null>(null);
const trashLoading = ref(false);
const restoring = ref<string | null>(null);

function conflictLabel(kind: string): string {
  return CONFLICT_LABELS[kind] ?? kind;
}

function report(kind: "error" | "ok", text: string): void {
  notice.value = { kind, text };
}

function countOf(library: LibraryDto, kind: string): number {
  const counts = library.counts;
  const key = kind as keyof typeof counts;
  return counts[key] ?? 0;
}

async function refresh(): Promise<void> {
  loading.value = true;
  failure.value = null;
  try {
    libraries.value = (await api.listLibraries()).items;
  } catch (error) {
    failure.value = describeError(error);
  } finally {
    loading.value = false;
  }
}

async function create(): Promise<void> {
  if (form.path.trim() === "") {
    report("error", "请填写目录路径");
    return;
  }
  try {
    await api.createLibrary({ path: form.path.trim() });
    report("ok", "媒体库已登记，扫描需要手动触发");
    form.path = "";
    await refresh();
  } catch (error) {
    report("error", describeError(error));
  }
}

async function scan(library: LibraryDto): Promise<void> {
  scanning.value = library.id;
  try {
    const task = await api.scanLibrary(library.id);
    report("ok", `扫描任务 #${task.id} 已提交，完成后刷新查看结果`);
    await refresh();
  } catch (error) {
    report("error", describeError(error));
  } finally {
    scanning.value = null;
  }
}

async function remove(library: LibraryDto): Promise<void> {
  try {
    await api.deleteLibrary(library.id);
    if (expanded.value === library.id) {
      expanded.value = null;
    }
    report("ok", "已移除登记；磁盘文件不受影响");
    await refresh();
  } catch (error) {
    report("error", describeError(error));
  }
}

async function toggleFiles(library: LibraryDto, kind: string | null): Promise<void> {
  if (expanded.value === library.id && expandedKind.value === kind) {
    expanded.value = null;
    expandedKind.value = null;
    expandedFiles.value = null;
    return;
  }
  expanded.value = library.id;
  expandedKind.value = kind;
  filesLoading.value = true;
  try {
    expandedFiles.value = await api.libraryFiles(library.id, kind);
  } catch (error) {
    report("error", describeError(error));
    expanded.value = null;
  } finally {
    filesLoading.value = false;
  }
}

async function loadPreview(library: LibraryDto): Promise<void> {
  if (previewLibrary.value === library.id) {
    previewLibrary.value = null;
    preview.value = null;
    return;
  }
  previewLibrary.value = library.id;
  previewLoading.value = true;
  preview.value = null;
  try {
    preview.value = await api.organizePreview(library.id);
  } catch (error) {
    report("error", describeError(error));
    previewLibrary.value = null;
  } finally {
    previewLoading.value = false;
  }
}

/**
 * 执行整理. 冲突项统一按所选处置处理, 无冲突项照常移动;
 * 目标路径由后端按当前配置重新渲染, 与预览同一套逻辑.
 */
async function execute(library: LibraryDto, action: string): Promise<void> {
  if (preview.value === null) {
    return;
  }
  executing.value = true;
  try {
    const choices: Record<string, string> = {};
    for (const item of preview.value.items) {
      if (item.conflict !== null && item.conflict !== undefined) {
        choices[item.source_path] = action;
      }
    }
    const task = await api.organizeExecute(library.id, choices);
    report("ok", `整理任务 #${task.id} 已提交，完成后刷新查看结果`);
    await loadPreview(library);
    await refresh();
  } catch (error) {
    report("error", describeError(error));
  } finally {
    executing.value = false;
  }
}

function formatSize(bytes: number): string {
  if (bytes < 1024) {
    return `${bytes} B`;
  }
  if (bytes < 1024 * 1024) {
    return `${(bytes / 1024).toFixed(1)} KB`;
  }
  if (bytes < 1024 * 1024 * 1024) {
    return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  }
  return `${(bytes / 1024 / 1024 / 1024).toFixed(2)} GB`;
}

async function loadCleanup(library: LibraryDto): Promise<void> {
  if (cleanupLibrary.value === library.id) {
    cleanupLibrary.value = null;
    cleanup.value = null;
    return;
  }
  cleanupLibrary.value = library.id;
  cleanupLoading.value = true;
  cleanup.value = null;
  try {
    cleanup.value = await api.cleanupManifest(library.id);
  } catch (error) {
    report("error", describeError(error));
    cleanupLibrary.value = null;
  } finally {
    cleanupLoading.value = false;
  }
}

async function runCleanup(library: LibraryDto): Promise<void> {
  cleaningUp.value = true;
  try {
    const task = await api.cleanupExecute(library.id);
    report("ok", `清理任务 #${task.id} 已提交，完成后刷新查看结果`);
    cleanup.value = null;
    cleanupLibrary.value = null;
  } catch (error) {
    report("error", describeError(error));
  } finally {
    cleaningUp.value = false;
  }
}

async function loadTrash(library: LibraryDto): Promise<void> {
  if (trashLibrary.value === library.id) {
    trashLibrary.value = null;
    trash.value = null;
    return;
  }
  trashLibrary.value = library.id;
  trashLoading.value = true;
  trash.value = null;
  try {
    trash.value = await api.trashListing(library.id);
  } catch (error) {
    report("error", describeError(error));
    trashLibrary.value = null;
  } finally {
    trashLoading.value = false;
  }
}

async function restore(library: LibraryDto, path: string): Promise<void> {
  restoring.value = path;
  try {
    trash.value = await api.trashRestore(library.id, path);
    report("ok", `已恢复 ${path}`);
  } catch (error) {
    report("error", describeError(error));
  } finally {
    restoring.value = null;
  }
}

onMounted(refresh);
</script>

<template>
  <div>
    <div class="flex items-center justify-between px-8 pt-5">
      <span class="font-mono text-xs text-neutral-500">
        {{ loading ? "读取中" : `${libraries.length} 个库` }}
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
      <section class="space-y-4">
        <p v-if="failure" class="rounded-md border border-red-900/60 bg-red-950/40 px-4 py-3 text-sm text-red-300">
          {{ failure }}
          <Button class="ml-3" variant="ghost" @click="refresh">重试</Button>
        </p>

        <p
          v-else-if="libraries.length === 0"
          class="rounded-lg border border-neutral-800/80 px-4 py-10 text-center text-sm text-neutral-600"
        >
          还没有媒体库。在右侧登记一个目录后派生扫描。
        </p>

        <article
          v-for="library in libraries"
          :key="library.id"
          class="rounded-lg border border-neutral-800/80"
        >
          <div class="flex items-start justify-between gap-4 px-4 py-3">
            <div class="min-w-0">
              <p class="truncate font-mono text-sm text-neutral-200">{{ library.path }}</p>
              <p class="mt-1 text-xs text-neutral-600">
                #{{ library.id }} · 登记于
                {{ new Date(library.created_at).toLocaleString("zh-CN", { hour12: false }) }}
              </p>
            </div>
            <div class="flex shrink-0 gap-2 text-xs">
              <Button
                variant="ghost"
                :disabled="scanning !== null"
                @click="scan(library)"
              >
                {{ scanning === library.id ? "提交中" : "扫描" }}
              </Button>
              <Button
                variant="ghost"
                :disabled="previewLoading"
                @click="loadPreview(library)"
              >
                {{ previewLibrary === library.id && previewLoading ? "预览中" : "整理预览" }}
              </Button>
              <button
                class="text-neutral-500 hover:text-neutral-300"
                @click="loadCleanup(library)"
              >
                清理
              </button>
              <button
                class="text-neutral-500 hover:text-neutral-300"
                @click="loadTrash(library)"
              >
                回收站
              </button>
              <button
                class="text-neutral-500 hover:text-red-400"
                @click="remove(library)"
              >
                移除
              </button>
            </div>
          </div>

          <div class="flex flex-wrap gap-2 border-t border-neutral-900 px-4 py-3">
            <button
              v-for="kind in KINDS"
              :key="kind.value"
              :class="[
                'rounded-full border px-3 py-1 text-xs',
                expanded === library.id && expandedKind === kind.value
                  ? 'border-emerald-600 bg-emerald-950/60 text-emerald-300'
                  : 'border-neutral-800 text-neutral-400 hover:text-neutral-200',
              ]"
              @click="toggleFiles(library, kind.value)"
            >
              {{ kind.label }} {{ countOf(library, kind.value) }}
            </button>
          </div>

          <div
            v-if="expanded === library.id"
            class="border-t border-neutral-900 px-4 py-3"
          >
            <p v-if="filesLoading" class="text-xs text-neutral-500">读取中…</p>
            <p
              v-else-if="(expandedFiles?.items.length ?? 0) === 0"
              class="text-xs text-neutral-600"
            >
              该类别下没有条目。
            </p>
            <ul v-else class="space-y-1 font-mono text-xs text-neutral-300">
              <li
                v-for="file in expandedFiles?.items"
                :key="file.path"
                class="flex flex-wrap items-baseline gap-2"
              >
                <span class="truncate">{{ file.path }}</span>
                <span
                  v-if="file.rule"
                  class="rounded bg-red-950/60 px-1.5 py-0.5 text-[10px] text-red-300"
                >
                  {{ file.rule }}
                </span>
                <span v-if="file.detail" class="text-neutral-600">{{ file.detail }}</span>
              </li>
            </ul>
          </div>

          <div
            v-if="previewLibrary === library.id"
            class="border-t border-neutral-900 px-4 py-3"
          >
            <p v-if="previewLoading" class="text-xs text-neutral-500">预览计算中…</p>
            <template v-else-if="preview">
              <p class="text-xs text-neutral-500">
                目标路径按当前模板渲染；预览不创建目录、不移动文件。
                <template v-if="preview.unmatched > 0">
                  {{ preview.unmatched }} 个文件提取不出番号，未参与整理。
                </template>
                <template v-if="preview.stale > 0">
                  {{ preview.stale }} 个索引条目在磁盘上已不存在。
                </template>
              </p>
              <p
                v-if="(preview.items.length ?? 0) === 0"
                class="mt-2 text-xs text-neutral-600"
              >
                没有可整理的文件。先扫描，再确认文件名里带番号。
              </p>
              <template v-else>
                <ul class="mt-2 space-y-2 font-mono text-xs">
                  <li
                    v-for="item in preview.items"
                    :key="item.source_path"
                    class="flex flex-wrap items-baseline gap-2"
                  >
                    <span class="text-neutral-500">{{ item.source_path }}</span>
                    <span class="text-neutral-600">→</span>
                    <span class="text-neutral-200">{{ item.target_path }}</span>
                    <span
                      v-if="item.conflict"
                      :class="[
                        'rounded px-1.5 py-0.5 text-[10px]',
                        item.conflict === 'same_path'
                          ? 'bg-neutral-800 text-neutral-400'
                          : 'bg-amber-950/60 text-amber-300',
                      ]"
                    >
                      {{ conflictLabel(item.conflict) }}
                    </span>
                  </li>
                </ul>
                <div class="mt-3 flex flex-wrap gap-2 text-xs">
                  <Button
                    variant="ghost"
                    :disabled="executing"
                    @click="execute(library, 'skip')"
                  >
                    执行（跳过冲突）
                  </Button>
                  <Button
                    variant="ghost"
                    :disabled="executing"
                    @click="execute(library, 'rename')"
                  >
                    执行（冲突改名）
                  </Button>
                  <Button
                    variant="ghost"
                    :disabled="executing"
                    @click="execute(library, 'overwrite')"
                  >
                    执行（覆盖冲突）
                  </Button>
                </div>
                <p class="mt-1 text-[10px] text-neutral-600">
                  覆盖前旧文件进入库内回收目录，可恢复。
                </p>
              </template>
            </template>
          </div>

          <div
            v-if="cleanupLibrary === library.id"
            class="border-t border-neutral-900 px-4 py-3"
          >
            <p v-if="cleanupLoading" class="text-xs text-neutral-500">读取中…</p>
            <template v-else-if="cleanup">
              <p
                v-if="(cleanup.items.length ?? 0) === 0"
                class="text-xs text-neutral-600"
              >
                没有待清理的文件。无效文件与系统产物会出现在这里。
              </p>
              <template v-else>
                <ul class="space-y-1 font-mono text-xs text-neutral-300">
                  <li
                    v-for="item in cleanup.items"
                    :key="item.path"
                    class="flex flex-wrap items-baseline gap-2"
                  >
                    <span class="truncate">{{ item.path }}</span>
                    <span class="text-neutral-600">{{ formatSize(item.size) }}</span>
                    <span class="rounded bg-amber-950/60 px-1.5 py-0.5 text-[10px] text-amber-300">
                      {{ item.reason }}
                    </span>
                  </li>
                </ul>
                <div class="mt-3 flex items-center gap-3 text-xs">
                  <Button
                    variant="ghost"
                    :disabled="cleaningUp"
                    @click="runCleanup(library)"
                  >
                    {{ cleaningUp ? "提交中" : "确认清理" }}
                  </Button>
                  <span class="text-neutral-600">
                    共 {{ cleanup.items.length }} 项，{{ formatSize(cleanup.total_size) }}
                  </span>
                </div>
                <p class="mt-1 text-[10px] text-neutral-600">
                  清理的文件进入库内回收目录，可在回收站恢复。
                </p>
              </template>
            </template>
          </div>

          <div
            v-if="trashLibrary === library.id"
            class="border-t border-neutral-900 px-4 py-3"
          >
            <p v-if="trashLoading" class="text-xs text-neutral-500">读取中…</p>
            <template v-else-if="trash">
              <p
                v-if="(trash.items.length ?? 0) === 0"
                class="text-xs text-neutral-600"
              >
                回收目录是空的。
              </p>
              <template v-else>
                <ul class="space-y-1 font-mono text-xs text-neutral-300">
                  <li
                    v-for="item in trash.items"
                    :key="item.path"
                    class="flex flex-wrap items-baseline gap-2"
                  >
                    <span class="truncate">{{ item.path }}</span>
                    <span class="text-neutral-600">{{ formatSize(item.size) }}</span>
                    <button
                      class="text-neutral-500 hover:text-emerald-400"
                      :disabled="restoring !== null"
                      @click="restore(library, item.path)"
                    >
                      {{ restoring === item.path ? "恢复中" : "恢复" }}
                    </button>
                  </li>
                </ul>
                <p class="mt-3 text-xs text-neutral-600">
                  回收目录占用：{{ formatSize(trash.total_size) }}。目标已存在时恢复会被拒绝。
                </p>
              </template>
            </template>
          </div>
        </article>
      </section>

      <aside>
        <form class="rounded-lg border border-neutral-800/80 p-4" @submit.prevent="create">
          <h2 class="text-sm font-medium text-neutral-200">登记媒体库</h2>
          <p class="mt-1 text-xs text-neutral-500">
            路径必须存在且可读；登记后点「扫描」归类文件。
          </p>
          <label class="mt-3 block text-xs text-neutral-500">
            目录路径
            <input
              v-model="form.path"
              class="mt-1 w-full rounded-md border border-neutral-800 bg-neutral-900 px-3 py-1.5 font-mono text-sm text-neutral-100 outline-none focus:border-emerald-600"
              placeholder="/Volumes/media/library"
            />
          </label>
          <Button class="mt-4 w-full" type="submit">登记</Button>
        </form>
      </aside>
    </main>
  </div>
</template>
