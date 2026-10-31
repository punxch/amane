<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { RouterLink, useRoute, useRouter } from "vue-router";

import {
  api,
  describeError,
  type MetadataDto,
  type MetadataFieldDto,
  type UpdateMetadataRequest,
} from "@/api/client";
import EmptyState from "@/components/common/EmptyState.vue";
import PageShell from "@/components/common/PageShell.vue";
import Alert from "@/components/ui/Alert.vue";
import Badge from "@/components/ui/Badge.vue";
import Button from "@/components/ui/Button.vue";
import Icon from "@/components/ui/Icon.vue";
import IconButton from "@/components/ui/IconButton.vue";
import Modal from "@/components/ui/Modal.vue";
import Paper from "@/components/ui/Paper.vue";
import TextInput from "@/components/ui/TextInput.vue";
import TextareaInput from "@/components/ui/TextareaInput.vue";
import Tooltip from "@/components/common/Tooltip.vue";
import { formatDate, formatTime } from "@/lib/format";

const route = useRoute();
const router = useRouter();

const number = computed(() => {
  const value = route.params.number;
  return typeof value === "string" ? value : "";
});

const detail = ref<MetadataDto | null>(null);
const loading = ref(false);
const failure = ref<string | null>(null);
const notice = ref<{ kind: "error" | "ok"; text: string } | null>(null);

const editing = ref<{ field: string; value: string } | null>(null);
const noteDraft = ref("");
const tagDraft = ref("");
const confirmOpen = ref(false);
const busy = ref(false);
/** 类型标识与展示名; 取不到时退回原始标识. */
const typeLabels = ref<Array<{ value: string; label: string }>>([]);

const pageTitle = computed(
  () => detail.value?.title ?? detail.value?.number ?? "影片详情",
);

function typeLabel(value: string): string {
  return typeLabels.value.find((option) => option.value === value)?.label ?? value;
}

function report(kind: "error" | "ok", text: string): void {
  notice.value = { kind, text };
}

function adopt(dto: MetadataDto): void {
  detail.value = dto;
  noteDraft.value = dto.note ?? "";
  editing.value = null;
}

async function load(): Promise<void> {
  if (number.value === "") {
    failure.value = "地址里没有番号";
    return;
  }
  loading.value = true;
  failure.value = null;
  try {
    // 类型展示名只是标签, 取不到也不应让详情页失败.
    const [dto, options] = await Promise.all([
      api.metadata(number.value),
      api.filters().catch(() => null),
    ]);
    if (options !== null) {
      typeLabels.value = options.content_types;
    }
    adopt(dto);
  } catch (error) {
    detail.value = null;
    failure.value = describeError(error);
  } finally {
    loading.value = false;
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

async function saveEdit(): Promise<void> {
  if (editing.value === null || detail.value === null) {
    return;
  }
  const { field, value } = editing.value;
  const payload = buildPayload(field, value);
  if (payload === null) {
    report("error", "时长应为整数分钟");
    return;
  }

  busy.value = true;
  try {
    adopt(await api.updateMetadata(detail.value.number, payload));
    report("ok", "已保存，该字段已锁定");
  } catch (error) {
    report("error", describeError(error));
  } finally {
    busy.value = false;
  }
}

/**
 * 单个字段的修订请求.
 *
 * 断言必要性: 字段名在运行时才确定 (来自服务端的逐字段视图), TypeScript 无法把动态键收窄到具体属性;
 * 收窄点集中在这一处, 键与 PATCH 的字段一一对应. 时长单独转成数字, 其余按文本提交.
 */
function buildPayload(field: string, value: string): UpdateMetadataRequest | null {
  if (field === "runtime_minutes") {
    const minutes = Number.parseInt(value.trim(), 10);
    if (Number.isNaN(minutes)) {
      return null;
    }
    return { runtime_minutes: minutes };
  }
  return { [field]: value } as UpdateMetadataRequest;
}

async function toggleLock(field: MetadataFieldDto): Promise<void> {
  if (detail.value === null) {
    return;
  }
  busy.value = true;
  try {
    adopt(await api.setLock(detail.value.number, field.field, !field.locked));
    report("ok", field.locked ? `已解锁${field.label}` : `已锁定${field.label}`);
  } catch (error) {
    report("error", describeError(error));
  } finally {
    busy.value = false;
  }
}

async function saveNote(): Promise<void> {
  if (detail.value === null) {
    return;
  }
  busy.value = true;
  try {
    adopt(await api.updateMetadata(detail.value.number, { note: noteDraft.value }));
    report("ok", "备注已保存");
  } catch (error) {
    report("error", describeError(error));
  } finally {
    busy.value = false;
  }
}

async function addTag(): Promise<void> {
  const name = tagDraft.value.trim();
  if (detail.value === null || name === "") {
    return;
  }
  busy.value = true;
  try {
    adopt(await api.addTag(detail.value.number, name));
    tagDraft.value = "";
  } catch (error) {
    report("error", describeError(error));
  } finally {
    busy.value = false;
  }
}

async function removeTag(name: string): Promise<void> {
  if (detail.value === null) {
    return;
  }
  busy.value = true;
  try {
    adopt(await api.removeTag(detail.value.number, name));
  } catch (error) {
    report("error", describeError(error));
  } finally {
    busy.value = false;
  }
}

/** 「站点 值」的列表, 用于按站存的评分与原站地址. */
function perSite(entries: Record<string, string | number> | undefined): string[] {
  return Object.entries(entries ?? {}).map(([site, value]) => `${site} ${value}`);
}

/** 删除前列出会连带删除的数据; 清单为空时只确认番号条目本身. */
const relatedSummary = computed(() => {
  const related = detail.value?.related;
  if (related === undefined) {
    return "";
  }
  const parts: string[] = [];
  if (related.actors > 0) parts.push(`演员 ${related.actors}`);
  if (related.sources > 0) parts.push(`来源快照 ${related.sources}`);
  if (related.locks > 0) parts.push(`字段锁 ${related.locks}`);
  if (related.tags > 0) parts.push(`标签 ${related.tags}`);
  if (related.resources > 0) parts.push(`本地资源 ${related.resources}`);
  return parts.join("、");
});

async function removeEntry(): Promise<void> {
  if (detail.value === null) {
    return;
  }
  busy.value = true;
  try {
    await api.deleteMetadata(detail.value.number);
    confirmOpen.value = false;
    await router.replace({ name: "metadata" });
  } catch (error) {
    report("error", describeError(error));
  } finally {
    busy.value = false;
  }
}

const subtitle = computed(() => {
  const item = detail.value;
  if (item === null) {
    return "";
  }
  return [item.number, item.maker, item.release_date ? formatDate(item.release_date) : null]
    .filter((part): part is string => part !== null && part !== "")
    .join(" · ");
});

watch(number, () => void load(), { immediate: true });
</script>

<template>
  <PageShell :title="pageTitle" :subtitle="subtitle">
    <template #actions>
      <RouterLink to="/">
        <Button variant="subtle" size="sm">返回片库</Button>
      </RouterLink>
      <Button variant="default" :disabled="loading" @click="load">刷新</Button>
      <Button v-if="detail !== null" variant="danger" size="sm" @click="confirmOpen = true">
        删除
      </Button>
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

    <EmptyState v-else-if="loading && detail === null" title="读取中…" />

    <div v-else-if="detail !== null" class="detail">
      <div class="detail__main">
        <Paper padding="none" class="detail__card">
          <header class="detail__card-head">
            <div class="detail__identity">
              <h2 class="detail__number">{{ detail.number }}</h2>
              <div class="detail__badges">
                <Badge tone="brand">{{ typeLabel(detail.content_type) }}</Badge>
                <Badge v-if="detail.locked_fields.length > 0" tone="warning">
                  {{ detail.locked_fields.length }} 个字段已锁定
                </Badge>
                <Badge v-if="detail.tags.length > 0" tone="gray">
                  {{ detail.tags.length }} 个标签
                </Badge>
              </div>
            </div>
            <p class="detail__updated">更新于 {{ formatTime(detail.updated_at) }}</p>
          </header>

          <p v-if="detail.title" class="detail__title">{{ detail.title }}</p>
          <p v-else class="detail__title detail__title--empty">没有标题</p>

          <table class="amane-table detail__fields">
            <thead>
              <tr>
                <th style="width: 132px">字段</th>
                <th>值</th>
                <th style="width: 148px; text-align: right">来源与操作</th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="field in detail.fields" :key="field.field">
                <td class="field-label">{{ field.label }}</td>
                <td>
                  <div v-if="editing?.field === field.field" class="field-editor">
                    <TextareaInput
                      v-if="field.field === 'plot'"
                      v-model="editing.value"
                      :rows="4"
                      aria-label="字段值"
                    />
                    <TextInput v-else v-model="editing.value" aria-label="字段值" />
                    <div class="field-editor__actions">
                      <Button variant="subtle" size="sm" @click="editing = null">取消</Button>
                      <Button size="sm" :loading="busy" @click="saveEdit">保存</Button>
                    </div>
                  </div>
                  <p v-else :class="['field-value', { 'field-value--empty': !field.value }]">
                    {{ field.value ?? "—" }}
                  </p>
                </td>
                <td>
                  <div class="field-actions">
                    <span
                      v-if="field.value !== null"
                      :class="[
                        'field-source',
                        field.source === 'manual' ? 'field-source--manual' : '',
                      ]"
                    >
                      {{ sourceLabel(field.source) }}
                    </span>
                    <Tooltip :label="field.locked ? '解锁字段' : '锁定字段'">
                      <IconButton
                        :label="field.locked ? '解锁字段' : '锁定字段'"
                        size="sm"
                        :disabled="busy"
                        @click="toggleLock(field)"
                      >
                        <Icon
                          :name="field.locked ? 'lock' : 'unlock'"
                          :size="15"
                          :class="field.locked ? 'icon-locked' : ''"
                        />
                      </IconButton>
                    </Tooltip>
                    <Tooltip v-if="field.field !== 'poster_url'" label="修订字段">
                      <IconButton
                        label="修订字段"
                        size="sm"
                        :disabled="busy"
                        @click="startEdit(field)"
                      >
                        <Icon name="pencil" :size="15" />
                      </IconButton>
                    </Tooltip>
                  </div>
                </td>
              </tr>
            </tbody>
          </table>
        </Paper>

        <div class="detail__grid">
          <Paper>
            <h3 class="section-title">标签</h3>
            <div class="tags">
              <span v-for="tag in detail.tags" :key="tag" class="tag">
                {{ tag }}
                <button type="button" aria-label="移除标签" @click="removeTag(tag)">
                  <Icon name="close" :size="12" />
                </button>
              </span>
              <span v-if="detail.tags.length === 0" class="tags__empty">还没有标签</span>
            </div>
            <form class="tag-form" @submit.prevent="addTag">
              <TextInput v-model="tagDraft" placeholder="新标签" size="sm" aria-label="新标签" />
              <Button size="sm" type="submit" :disabled="busy">添加</Button>
            </form>
          </Paper>

          <Paper>
            <div class="section-head">
              <h3 class="section-title">备注</h3>
              <Button variant="subtle" size="sm" :loading="busy" @click="saveNote">保存</Button>
            </div>
            <TextareaInput
              v-model="noteDraft"
              :rows="4"
              placeholder="写给自己的备注"
              aria-label="备注"
            />
          </Paper>
        </div>
      </div>

      <aside class="detail__aside">
        <Paper padding="sm">
          <div class="cover">
            <img
              v-if="detail.cover_local_url"
              :src="detail.cover_local_url"
              :alt="detail.number"
            />
            <img
              v-else-if="detail.poster_local_url"
              :src="detail.poster_local_url"
              :alt="detail.number"
              class="cover__poster"
            />
            <div v-else class="cover__placeholder">
              <Icon name="movie" :size="30" :stroke="1.3" />
              <span>海报尚未存到本地</span>
            </div>
          </div>
          <p v-if="detail.poster_url && !detail.poster_local_url" class="cover__hint">
            远端海报：{{ detail.poster_url }}
          </p>
        </Paper>

        <Paper>
          <h3 class="section-title">影片信息</h3>
          <dl class="facts">
            <div>
              <dt>类型</dt>
              <dd>{{ typeLabel(detail.content_type) }}</dd>
            </div>
            <div>
              <dt>制作商</dt>
              <dd>{{ detail.maker ?? "—" }}</dd>
            </div>
            <div>
              <dt>发行日期</dt>
              <dd>{{ formatDate(detail.release_date) }}</dd>
            </div>
            <div>
              <dt>时长</dt>
              <dd>{{ detail.runtime_minutes === null ? "—" : `${detail.runtime_minutes} 分钟` }}</dd>
            </div>
            <div v-if="detail.series">
              <dt>系列</dt>
              <dd>{{ detail.series }}</dd>
            </div>
            <div>
              <dt>创建</dt>
              <dd>{{ formatTime(detail.created_at) }}</dd>
            </div>
          </dl>

          <template v-if="detail.genres.length > 0">
            <h4 class="facts__title">题材</h4>
            <div class="chips">
              <span v-for="item in detail.genres" :key="item" class="chip">{{ item }}</span>
            </div>
          </template>

          <template v-if="detail.actors.length > 0">
            <h4 class="facts__title">演员</h4>
            <div class="chips">
              <span v-for="item in detail.actors" :key="item" class="chip">{{ item }}</span>
            </div>
          </template>

          <template v-if="perSite(detail.scores).length > 0">
            <h4 class="facts__title">评分</h4>
            <p class="facts__line">{{ perSite(detail.scores).join("、") }}</p>
          </template>

          <template v-if="detail.sources.length > 0">
            <h4 class="facts__title">命中来源</h4>
            <p class="facts__line">{{ detail.sources.join(" / ") }}</p>
          </template>

          <template v-if="Object.keys(detail.source_urls).length > 0">
            <h4 class="facts__title">原站</h4>
            <div class="links">
              <a
                v-for="[site, url] in Object.entries(detail.source_urls)"
                :key="site"
                :href="url"
                target="_blank"
                rel="noreferrer"
                class="link"
              >
                {{ site }}
                <Icon name="external" :size="12" />
              </a>
            </div>
          </template>

          <template v-if="detail.trailer_urls.length > 0">
            <h4 class="facts__title">预告</h4>
            <div class="links">
              <a
                v-for="(url, index) in detail.trailer_urls"
                :key="url"
                :href="url"
                target="_blank"
                rel="noreferrer"
                class="link"
              >
                预告 {{ index + 1 }}
                <Icon name="external" :size="12" />
              </a>
            </div>
          </template>
        </Paper>
      </aside>
    </div>

    <Modal :opened="confirmOpen" title="删除影片" :width="420" @close="confirmOpen = false">
      <p class="confirm-text">
        删除 <strong>{{ detail?.number }}</strong> 会同时删除{{ relatedSummary === "" ? "该影片" : `：${relatedSummary}` }}。
      </p>
      <p class="confirm-hint">磁盘上的媒体文件不受影响，删除后无法撤销。</p>
      <template #footer>
        <Button variant="default" @click="confirmOpen = false">取消</Button>
        <Button variant="danger" :loading="busy" @click="removeEntry">删除</Button>
      </template>
    </Modal>
  </PageShell>
</template>

<style scoped>
.detail {
  display: grid;
  grid-template-columns: minmax(0, 1fr);
  gap: 16px;
  align-items: start;
}

@media (min-width: 62em) {
  .detail {
    grid-template-columns: minmax(0, 1fr) 320px;
  }
}

.detail__main {
  display: flex;
  flex-direction: column;
  gap: 16px;
  min-width: 0;
}

.detail__aside {
  display: flex;
  flex-direction: column;
  gap: 16px;
  min-width: 0;
}

.detail__card-head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
  padding: 16px 16px 0;
  flex-wrap: wrap;
}

.detail__identity {
  display: flex;
  flex-direction: column;
  gap: 6px;
  min-width: 0;
}

.detail__number {
  font-family: var(--font-mono);
  font-size: 20px;
}

.detail__badges {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}

.detail__updated {
  margin: 0;
  color: var(--amane-subtle);
  font-family: var(--font-mono);
  font-size: 12px;
}

.detail__title {
  margin: 12px 16px 16px;
  color: var(--amane-fg);
  font-size: 15px;
  line-height: 1.5;
}

.detail__title--empty {
  color: var(--amane-subtle);
}

.detail__fields {
  border-top: 1px solid var(--amane-line);
}

.field-label {
  color: var(--amane-dimmed);
  font-size: 13px;
  white-space: nowrap;
}

.field-value {
  margin: 0;
  color: var(--amane-fg);
  font-size: 13px;
  line-height: 1.6;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}

.field-value--empty {
  color: var(--amane-subtle);
}

.field-actions {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 2px;
}

.field-source {
  margin-right: 2px;
  color: var(--amane-subtle);
  font-size: 11px;
}

.field-source--manual {
  color: var(--amane-yellow);
}

.icon-locked {
  color: var(--amane-yellow);
}

.field-editor {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.field-editor__actions {
  display: flex;
  justify-content: flex-end;
  gap: 6px;
}

.detail__grid {
  display: grid;
  grid-template-columns: minmax(0, 1fr);
  gap: 16px;
}

@media (min-width: 62em) {
  .detail__grid {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
}

.section-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  margin-bottom: 8px;
}

.section-title {
  margin: 0 0 8px;
  font-size: 14px;
}

.section-head .section-title {
  margin-bottom: 0;
}

.tags {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin-bottom: 10px;
}

.tag {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 2px 6px 2px 8px;
  border-radius: 4px;
  background: var(--amane-surface-2);
  color: var(--amane-fg);
  font-size: 12px;
}

.tag button {
  display: inline-flex;
  border: 0;
  padding: 0;
  background: transparent;
  color: var(--amane-dimmed);
  cursor: pointer;
}

.tag button:hover {
  color: var(--amane-red);
}

.tags__empty {
  color: var(--amane-subtle);
  font-size: 13px;
}

.tag-form {
  display: flex;
  gap: 8px;
}

.cover {
  overflow: hidden;
  border-radius: var(--amane-radius);
  background: var(--amane-surface-2);
}

.cover img {
  display: block;
  width: 100%;
  height: auto;
}

.cover__poster {
  max-height: 420px;
  object-fit: cover;
}

.cover__placeholder {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 8px;
  aspect-ratio: 2 / 3;
  color: var(--amane-subtle);
  font-size: 12px;
}

.cover__hint {
  margin: 8px 0 0;
  overflow: hidden;
  color: var(--amane-yellow);
  font-family: var(--font-mono);
  font-size: 11px;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.facts {
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin: 0;
}

.facts > div {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 12px;
}

.facts dt {
  color: var(--amane-dimmed);
  font-size: 13px;
}

.facts dd {
  margin: 0;
  color: var(--amane-fg);
  font-size: 13px;
  text-align: right;
  overflow-wrap: anywhere;
}

.facts__title {
  margin: 14px 0 6px;
  color: var(--amane-dimmed);
  font-size: 12px;
  font-weight: 600;
}

.facts__line {
  margin: 0;
  color: var(--amane-fg);
  font-size: 13px;
  overflow-wrap: anywhere;
}

.chips {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}

.chip {
  padding: 2px 8px;
  border-radius: 4px;
  background: var(--amane-surface-2);
  color: var(--amane-fg);
  font-size: 12px;
}

.links {
  display: flex;
  flex-wrap: wrap;
  gap: 10px;
}

.link {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  color: var(--amane-brand-text);
  font-size: 13px;
}

.link:hover {
  text-decoration: underline;
}

.confirm-text {
  margin: 0;
  font-size: 14px;
}

.confirm-hint {
  margin: 0;
  color: var(--amane-dimmed);
  font-size: 13px;
}
</style>
