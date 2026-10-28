<script setup lang="ts">
import { onMounted, reactive, ref } from "vue";

import {
  ApiError,
  describeError,
  api,
  type ConfigState,
  type HotSettingsDto,
  type SettingsDto,
  type UpdateConfigRequest,
} from "@/api/client";
import Button from "@/components/ui/Button.vue";

interface FieldSchema {
  type?: string;
  description?: string;
  default?: unknown;
  minimum?: number;
  maximum?: number;
  "x-widget"?: string;
  items?: { type?: string };
  $ref?: string;
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
/** 最近一次读到的配置; 提交时作为基底, 表单没渲染的字段原样带回. */
const loaded = ref<SettingsDto | null>(null);
const form = reactive<{ cold: Record<string, string>; hot: Record<string, unknown> }>({
  cold: {},
  hot: {},
});

function report(kind: "error" | "ok", text: string): void {
  notice.value = { kind, text };
}

function widgetOf(spec: FieldSchema): string {
  return spec["x-widget"] ?? "text";
}

/**
 * 取某一层的可编辑字段.
 *
 * 只列有控件的标量: 嵌套对象 (扫描规则、整理规则) 由各自的页面维护, 在这里渲染成
 * 一个没有控件的标签只会让人以为它能改. 新增配置项只需要改后端 schema.
 */
function fields(layer: "HotSettings" | "ColdSettings"): [string, FieldSchema][] {
  const section = schema.value[layer] ?? {};
  return Object.entries(section.properties ?? {}).filter(
    ([, spec]) => spec.$ref === undefined && spec.type !== "object",
  );
}

function label(field: string, spec: FieldSchema): string {
  return spec.description?.split(".")[0] ?? field;
}

/** 列表控件在表单里是逗号分隔的文本, 其余控件直接取原值. */
function hotText(field: string): string {
  const value = form.hot[field];
  if (Array.isArray(value)) {
    return value.join(", ");
  }
  return value === undefined || value === null ? "" : String(value);
}

function parseList(text: string): string[] {
  return text
    .split(",")
    .map((item) => item.trim())
    .filter((item) => item !== "");
}

function writeHot(field: string, widget: string, raw: string): void {
  if (widget === "list") {
    form.hot[field] = parseList(raw);
    return;
  }
  form.hot[field] = widget === "text" ? raw : Number(raw);
}

function writeCold(field: string, raw: string): void {
  form.cold[field] = raw;
}

/**
 * 取输入框的值.
 *
 * 断言必要性: 事件只挂在校验过的 `input` 元素上, 但 DOM 事件目标的类型无法静态收窄.
 */
function inputValue(event: Event): string {
  return (event.target as HTMLInputElement).value;
}

function numberAt(field: string): number {
  const value = form.hot[field];
  return typeof value === "number" ? value : Number(value ?? 0);
}

function listAt(field: string): string[] {
  const value = form.hot[field];
  return Array.isArray(value) ? value.map(String) : [];
}

function buildPayload(): UpdateConfigRequest {
  const base = loaded.value;
  if (base === null) {
    throw new ApiError("unreachable", "配置尚未读取成功");
  }

  // 未在表单里出现的字段 (扫描与整理规则) 原样带回: 漏掉它们等于把用户设置重置成默认值.
  const hot: HotSettingsDto = {
    ...base.hot,
    concurrency: numberAt("concurrency"),
    request_timeout_secs: numberAt("request_timeout_secs"),
    enabled_sources: listAt("enabled_sources"),
  };
  const token = (form.cold.api_token ?? "").trim();

  return {
    cold: {
      data_dir: (form.cold.data_dir ?? "").trim(),
      bind: (form.cold.bind ?? "").trim(),
      api_token: token === "" ? null : token,
    },
    hot,
  };
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
    loaded.value = state;

    form.cold = {
      data_dir: state.cold.data_dir,
      bind: state.cold.bind,
      api_token: state.cold.api_token ?? "",
    };
    form.hot = { ...state.hot };
  } catch (error) {
    failure.value = describeError(error);
  } finally {
    loading.value = false;
  }
}

async function submit(): Promise<void> {
  applied.value = [];
  restartRequired.value = [];
  try {
    const state: ConfigState = await api.saveConfig(buildPayload());

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
    loaded.value = state.settings;
    form.cold = {
      data_dir: state.settings.cold.data_dir,
      bind: state.settings.cold.bind,
      api_token: state.settings.cold.api_token ?? "",
    };
    form.hot = { ...state.settings.hot };
  } catch (error) {
    report("error", describeError(error));
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
            v-if="widgetOf(spec) === 'slider'"
            :value="hotText(field)"
            type="range"
            class="mt-2 w-full"
            :min="spec.minimum ?? 0"
            :max="spec.maximum ?? 100"
            @input="writeHot(field, 'slider', inputValue($event))"
          />
          <input
            v-else-if="widgetOf(spec) === 'number'"
            :value="hotText(field)"
            type="number"
            class="mt-2 w-full rounded-md border border-neutral-800 bg-neutral-900 px-3 py-1.5 text-sm text-neutral-100 outline-none focus:border-emerald-600"
            :min="spec.minimum"
            :max="spec.maximum"
            @input="writeHot(field, 'number', inputValue($event))"
          />
          <input
            v-else-if="widgetOf(spec) === 'list'"
            :value="hotText(field)"
            class="mt-2 w-full rounded-md border border-neutral-800 bg-neutral-900 px-3 py-1.5 font-mono text-sm text-neutral-100 outline-none focus:border-emerald-600"
            placeholder="dmm, javdb"
            @input="writeHot(field, 'list', inputValue($event))"
          />
          <input
            v-else
            :value="hotText(field)"
            type="text"
            class="mt-2 w-full rounded-md border border-neutral-800 bg-neutral-900 px-3 py-1.5 text-sm text-neutral-100 outline-none focus:border-emerald-600"
            @input="writeHot(field, 'text', inputValue($event))"
          />

          <p v-if="widgetOf(spec) === 'slider'" class="mt-1 font-mono text-xs text-neutral-400">
            {{ hotText(field) }}
          </p>
          <p v-if="widgetOf(spec) === 'list'" class="mt-1 text-xs text-neutral-600">
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
            :value="form.cold[field] ?? ''"
            type="text"
            class="mt-2 w-full rounded-md border border-neutral-800 bg-neutral-900 px-3 py-1.5 font-mono text-sm text-neutral-100 outline-none focus:border-emerald-600"
            @input="writeCold(field, inputValue($event))"
          />
        </div>
      </section>
    </main>
  </div>
</template>
