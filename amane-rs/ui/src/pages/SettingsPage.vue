<script setup lang="ts">
import { computed, onMounted, reactive, ref } from "vue";

import {
  ApiError,
  api,
  describeError,
  type ConfigState,
  type HotSettingsDto,
  type SettingsDto,
  type UpdateConfigRequest,
} from "@/api/client";
import PageShell from "@/components/common/PageShell.vue";
import Alert from "@/components/ui/Alert.vue";
import Button from "@/components/ui/Button.vue";
import Paper from "@/components/ui/Paper.vue";
import TextInput from "@/components/ui/TextInput.vue";

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
const saving = ref(false);
const failure = ref<string | null>(null);
const notice = ref<{ kind: "error" | "ok"; text: string } | null>(null);
const applied = ref<string[]>([]);
const restartRequired = ref<string[]>([]);
const dirty = ref(false);

const schema = ref<Record<string, SectionSchema>>({});
/** 内容类型与其展示名; 由筛选选项接口给出, 界面不自行维护枚举. */
const contentTypes = ref<Array<{ value: string; label: string }>>([]);
/** 最近一次读到的配置; 提交时作为基底, 表单没渲染的字段原样带回. */
const loaded = ref<SettingsDto | null>(null);
/** 类型路由单独存放: 它是已知形状的对象, 放在 `unknown` 的表单里会逼出断言. */
const routes = ref<Record<string, string[]>>({});
const form = reactive<{ cold: Record<string, string>; hot: Record<string, unknown> }>({
  cold: {},
  hot: {},
});

const subtitle = computed(() => {
  if (loading.value) {
    return "读取中";
  }
  return dirty.value ? "有未保存的更改" : "热配置立即生效，冷配置需要重启";
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
 * 只列有控件的标量: 嵌套对象 (扫描规则、整理规则) 由各自的页面维护, 在这里渲染成一个没有控件的标签
 * 只会让人以为它能改. 新增配置项只需要改后端 schema.
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
  dirty.value = true;
  if (widget === "list") {
    form.hot[field] = parseList(raw);
    return;
  }
  form.hot[field] = widget === "text" ? raw : Number(raw);
}

function writeCold(field: string, raw: string): void {
  dirty.value = true;
  form.cold[field] = raw;
}

/** 取值经事件目标收窄, 而不是把 DOM 事件断言成 Input 元素. */
function inputValue(event: Event): string {
  const target = event.target;
  return target instanceof HTMLInputElement ? target.value : "";
}

function numberAt(field: string): number {
  const value = form.hot[field];
  return typeof value === "number" ? value : Number(value ?? 0);
}

/** 某一类型的来源链在表单里是逗号分隔的文本. */
function chainText(contentType: string): string {
  return (routes.value[contentType] ?? []).join(", ");
}

function writeChain(contentType: string, raw: string): void {
  dirty.value = true;
  routes.value = { ...routes.value, [contentType]: parseList(raw) };
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
    content_routes: routes.value,
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
    const [state, document, options] = await Promise.all([
      api.readConfig(),
      api.configSchema(),
      api.filters(),
    ]);
    contentTypes.value = options.content_types;
    // 断言必要性: schema 来自服务端, 这里只按 JSON 文档结构读取, 不做运行时校验.
    schema.value = (document as { $defs?: Record<string, SectionSchema> }).$defs ?? {};
    loaded.value = state;

    form.cold = {
      data_dir: state.cold.data_dir,
      bind: state.cold.bind,
      api_token: state.cold.api_token ?? "",
    };
    form.hot = { ...state.hot };
    routes.value = state.hot.content_routes;
    dirty.value = false;
  } catch (error) {
    failure.value = describeError(error);
  } finally {
    loading.value = false;
  }
}

async function submit(): Promise<void> {
  applied.value = [];
  restartRequired.value = [];
  saving.value = true;
  try {
    const state: ConfigState = await api.saveConfig(buildPayload());

    applied.value = state.applied;
    restartRequired.value = state.restart_required;
    if (restartRequired.value.length > 0) {
      report("ok", `以下修改需要重启进程后生效: ${restartRequired.value.join("、")}`);
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
    routes.value = state.settings.hot.content_routes;
    dirty.value = false;
  } catch (error) {
    report("error", describeError(error));
  } finally {
    saving.value = false;
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
  <PageShell title="设置" :subtitle="subtitle">
    <template #actions>
      <Button variant="default" :disabled="loading" @click="reset">重置</Button>
      <Button :loading="saving" :disabled="loading" @click="submit">保存</Button>
    </template>

    <Alert v-if="notice !== null" :tone="notice.kind === 'error' ? 'danger' : 'success'">
      {{ notice.text }}
    </Alert>

    <Alert v-if="failure !== null" tone="danger">
      {{ failure }}
      <template #action>
        <Button variant="subtle" size="sm" @click="load">重试</Button>
      </template>
    </Alert>

    <div class="settings">
      <Paper class="settings__section">
        <h2 class="settings__title">热配置</h2>
        <p class="settings__hint">保存后立即生效，正在运行的任务不受影响。</p>

        <div v-for="[field, spec] in fields('HotSettings')" :key="field" class="field">
          <label class="field__label" :for="`hot-${field}`">
            {{ label(field, spec) }}
            <span class="field__key">{{ field }}</span>
            <span v-if="spec.minimum !== undefined" class="field__range">
              {{ spec.minimum }}–{{ spec.maximum }}
            </span>
          </label>

          <input
            v-if="widgetOf(spec) === 'slider'"
            :id="`hot-${field}`"
            :value="hotText(field)"
            class="slider"
            type="range"
            :min="spec.minimum ?? 0"
            :max="spec.maximum ?? 100"
            @input="writeHot(field, 'slider', inputValue($event))"
          />
          <TextInput
            v-else
            :model-value="hotText(field)"
            :type="widgetOf(spec) === 'number' ? 'number' : 'text'"
            :mono="widgetOf(spec) === 'list'"
            :aria-label="label(field, spec)"
            @update:model-value="(value) => writeHot(field, widgetOf(spec), value)"
          />

          <p v-if="widgetOf(spec) === 'slider'" class="field__value">{{ hotText(field) }}</p>
          <p v-if="widgetOf(spec) === 'list'" class="field__note">逗号分隔。</p>
        </div>

        <div class="settings__divider">
          <h3 class="settings__subtitle">类型路由</h3>
          <p class="settings__hint">
            每类影片按链上顺序询问来源，第一个给出值的来源决定该字段。链不能为空。
          </p>

          <div v-for="option in contentTypes" :key="option.value" class="field">
            <label class="field__label" :for="`route-${option.value}`">
              {{ option.label }}
              <span class="field__key">{{ option.value }}</span>
            </label>
            <TextInput
              :model-value="chainText(option.value)"
              mono
              placeholder="dmm, javdb"
              :aria-label="option.label"
              @update:model-value="(value) => writeChain(option.value, value)"
            />
          </div>
        </div>
      </Paper>

      <Paper class="settings__section">
        <h2 class="settings__title">冷配置</h2>
        <p class="settings__hint">保存后需要重启进程才能生效，界面会列出未生效的项。</p>

        <div v-for="[field, spec] in fields('ColdSettings')" :key="field" class="field">
          <label class="field__label" :for="`cold-${field}`">
            {{ label(field, spec) }}
            <span class="field__key">{{ field }}</span>
          </label>
          <TextInput
            :model-value="form.cold[field] ?? ''"
            mono
            :aria-label="label(field, spec)"
            @update:model-value="(value) => writeCold(field, value)"
          />
        </div>
      </Paper>
    </div>

    <Transition name="save-bar">
      <div v-if="dirty" class="save-bar">
        <span>有未保存的更改</span>
        <div class="save-bar__actions">
          <Button variant="default" size="sm" :disabled="saving" @click="reset">放弃</Button>
          <Button size="sm" :loading="saving" @click="submit">保存</Button>
        </div>
      </div>
    </Transition>
  </PageShell>
</template>

<style scoped>
.settings {
  display: grid;
  grid-template-columns: minmax(0, 1fr);
  gap: 16px;
  align-items: start;
}

@media (min-width: 62em) {
  .settings {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
}

.settings__section {
  min-width: 0;
}

.settings__title {
  font-size: 15px;
}

.settings__subtitle {
  margin: 0;
  font-size: 14px;
}

.settings__hint {
  margin: 4px 0 0;
  color: var(--amane-dimmed);
  font-size: 13px;
  line-height: 1.6;
}

.settings__divider {
  margin-top: 20px;
  padding-top: 16px;
  border-top: 1px solid var(--amane-line);
}

.field {
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin-top: 14px;
}

.field__label {
  display: flex;
  align-items: baseline;
  gap: 8px;
  color: var(--amane-dimmed);
  font-size: 12px;
}

.field__key {
  color: var(--amane-subtle);
  font-family: var(--font-mono);
  font-size: 11px;
}

.field__range {
  margin-left: auto;
  color: var(--amane-subtle);
  font-size: 11px;
}

.field__value {
  margin: 0;
  color: var(--amane-fg);
  font-family: var(--font-mono);
  font-size: 12px;
}

.field__note {
  margin: 0;
  color: var(--amane-subtle);
  font-size: 12px;
}

.slider {
  width: 100%;
  accent-color: var(--amane-brand);
}

.save-bar {
  position: sticky;
  bottom: 0;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  margin-top: 16px;
  padding: 10px 14px;
  border: 1px solid var(--amane-line);
  border-radius: var(--amane-radius);
  background: var(--amane-surface-2);
  color: var(--amane-fg);
  font-size: 13px;
}

.save-bar__actions {
  display: flex;
  gap: 8px;
}

.save-bar-enter-active,
.save-bar-leave-active {
  transition:
    opacity 120ms ease,
    transform 120ms ease;
}

.save-bar-enter-from,
.save-bar-leave-to {
  opacity: 0;
  transform: translateY(6px);
}
</style>
