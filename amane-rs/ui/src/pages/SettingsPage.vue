<script setup lang="ts">
import { onMounted, reactive, ref } from "vue";

import { ApiError, api, type ConfigState } from "@/api/client";
import Button from "@/components/ui/Button.vue";

interface FieldSchema {
  type?: string;
  description?: string;
  default?: unknown;
  minimum?: number;
  maximum?: number;
  "x-widget"?: string;
  items?: { type?: string };
}

interface SectionSchema {
  properties?: Record<string, FieldSchema>;
}

const loading = ref(false);
const failure = ref<string | null>(null);
const notice = ref<{ kind: "error" | "ok"; text: string } | null>(null);
const applied = ref<string[]>([]);
const restartRequired = ref<string[]>([]);

const schema = ref<Record<string, SectionSchema>>({});
const form = reactive({
  cold: { data_dir: "", bind: "", api_token: "" },
  hot: { concurrency: 4, request_timeout_secs: 20, enabled_sources: "" },
});

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

/** 取某一层的字段定义; 新增配置项只需要改后端 schema.
 *
 * 控件按 `x-widget` 分派: slider / number / list. 出现未知取值时不渲染该字段,
 * 而不是拿一个语义不符的控件硬套.
 */
function fields(layer: "HotSettings" | "ColdSettings"): [string, FieldSchema][] {
  const section = schema.value[layer] ?? {};
  return Object.entries(section.properties ?? {});
}

function label(field: string, spec: FieldSchema): string {
  return spec.description?.split(".")[0] ?? field;
}

async function load(): Promise<void> {
  loading.value = true;
  failure.value = null;
  try {
    const [state, document] = await Promise.all([
      api.readConfig(),
      api.configSchema(),
    ]);
    // 断言必要性: schema 来自服务端, 这里只按 JSON 文档结构读取, 不做运行时校验.
    schema.value = (document as { $defs?: Record<string, SectionSchema> }).$defs ?? {};

    form.cold.data_dir = state.cold.data_dir;
    form.cold.bind = state.cold.bind;
    form.cold.api_token = state.cold.api_token ?? "";
    form.hot.concurrency = state.hot.concurrency;
    form.hot.request_timeout_secs = state.hot.request_timeout_secs;
    form.hot.enabled_sources = state.hot.enabled_sources.join(", ");
  } catch (error) {
    failure.value = describe(error);
  } finally {
    loading.value = false;
  }
}

/** 由逗号分隔的文本还原为列表; 空串表示未启用任何限制. */
function parseSources(text: string): string[] {
  return text
    .split(",")
    .map((item) => item.trim())
    .filter((item) => item !== "");
}

async function submit(): Promise<void> {
  applied.value = [];
  restartRequired.value = [];
  try {
    const state: ConfigState = await api.saveConfig({
      cold: {
        data_dir: form.cold.data_dir.trim(),
        bind: form.cold.bind.trim(),
        api_token: form.cold.api_token.trim() === "" ? null : form.cold.api_token.trim(),
      },
      hot: {
        concurrency: form.hot.concurrency,
        request_timeout_secs: form.hot.request_timeout_secs,
        enabled_sources: parseSources(form.hot.enabled_sources),
      },
    });

    applied.value = state.applied;
    restartRequired.value = state.restart_required;
    if (restartRequired.value.length > 0) {
      report(
        "ok",
        `以下修改需要重启进程后生效: ${restartRequired.value.join("、")}`,
      );
    } else if (applied.value.length > 0) {
      report("ok", `已生效: ${applied.value.join("、")}`);
    } else {
      report("ok", "配置已保存, 没有需要生效的改动");
    }

    // 回显服务端的当前值, 冷配置未生效时表单保持真实状态.
    form.cold.data_dir = state.settings.cold.data_dir;
    form.cold.bind = state.settings.cold.bind;
    form.hot.concurrency = state.settings.hot.concurrency;
    form.hot.request_timeout_secs = state.settings.hot.request_timeout_secs;
    form.hot.enabled_sources = state.settings.hot.enabled_sources.join(", ");
  } catch (error) {
    report("error", describe(error));
  }
}

function reset(): void {
  applied.value = [];
  restartRequired.value = [];
  notice.value = null;
  void load();
}

onMounted(load);
</script>

<template>
  <div>
    <div class="flex items-center justify-between px-8 pt-5">
      <span class="font-mono text-xs text-neutral-500">
        {{ loading ? "读取中" : "配置" }}
      </span>
      <div class="flex gap-2">
        <Button variant="ghost" :disabled="loading" @click="reset">重置</Button>
        <Button :disabled="loading" @click="submit">保存</Button>
      </div>
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

    <p v-if="failure" class="mx-8 mt-4 rounded-md border border-red-900/60 bg-red-950/40 px-4 py-2 text-sm text-red-300">
      {{ failure }}
      <Button class="ml-3" variant="ghost" @click="load">重试</Button>
    </p>

    <main class="grid items-start gap-6 px-8 py-6 lg:grid-cols-2">
      <section class="rounded-lg border border-neutral-800/80 p-5">
        <h2 class="text-sm font-medium text-neutral-200">热配置</h2>
        <p class="mt-1 text-xs text-neutral-500">保存后立即生效，正在运行的任务不受影响。</p>

        <div v-for="[field, spec] in fields('HotSettings')" :key="field" class="mt-4">
          <label class="block text-xs text-neutral-500">
            {{ label(field, spec) }}
            <span class="ml-1 font-mono text-neutral-700">{{ field }}</span>
            <span v-if="spec.minimum !== undefined" class="ml-2 text-neutral-700">
              {{ spec.minimum }}–{{ spec.maximum }}
            </span>
          </label>

          <input
            v-if="(spec['x-widget'] ?? 'text') === 'slider'"
            :value="String(form.hot[field as keyof typeof form.hot])"
            type="range"
            class="mt-2 w-full"
            :min="spec.minimum ?? 0"
            :max="spec.maximum ?? 100"
            @input="form.hot[field as 'concurrency'] = Number(($event.target as HTMLInputElement).value)"
          />
          <input
            v-else-if="(spec['x-widget'] ?? 'text') === 'number'"
            :value="String(form.hot[field as keyof typeof form.hot])"
            type="number"
            class="mt-2 w-full rounded-md border border-neutral-800 bg-neutral-900 px-3 py-1.5 text-sm text-neutral-100 outline-none focus:border-emerald-600"
            :min="spec.minimum"
            :max="spec.maximum"
            @input="form.hot[field as 'request_timeout_secs'] = Number(($event.target as HTMLInputElement).value)"
          />
          <input
            v-else-if="(spec['x-widget'] ?? 'text') === 'list'"
            v-model="form.hot.enabled_sources"
            class="mt-2 w-full rounded-md border border-neutral-800 bg-neutral-900 px-3 py-1.5 font-mono text-sm text-neutral-100 outline-none focus:border-emerald-600"
            placeholder="dmm, javdb"
          />

          <p v-if="(spec['x-widget'] ?? 'text') === 'slider'" class="mt-1 font-mono text-xs text-neutral-400">
            {{ form.hot[field as "concurrency"] }}
          </p>
          <p v-if="(spec['x-widget'] ?? 'text') === 'list'" class="mt-1 text-xs text-neutral-600">
            逗号分隔，顺序即聚合优先级；未列出的来源不参与，留空表示全部启用。
          </p>
        </div>
      </section>

      <section class="rounded-lg border border-neutral-800/80 p-5">
        <h2 class="text-sm font-medium text-neutral-200">冷配置</h2>
        <p class="mt-1 text-xs text-neutral-500">保存后需要重启进程才能生效，界面会列出未生效的项。</p>

        <div v-for="[field, spec] in fields('ColdSettings')" :key="field" class="mt-4">
          <label class="block text-xs text-neutral-500">
            {{ label(field, spec) }}
            <span class="ml-1 font-mono text-neutral-700">{{ field }}</span>
          </label>
          <input
            :value="form.cold[field as keyof typeof form.cold]"
            type="text"
            class="mt-2 w-full rounded-md border border-neutral-800 bg-neutral-900 px-3 py-1.5 font-mono text-sm text-neutral-100 outline-none focus:border-emerald-600"
            @input="form.cold[field as 'bind'] = ($event.target as HTMLInputElement).value"
          />
        </div>
      </section>
    </main>
  </div>
</template>
