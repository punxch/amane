<script setup lang="ts">
import { computed } from "vue";

const props = defineProps<{ text: string; keyword: string }>();

interface Segment {
  text: string;
  hit: boolean;
}

/**
 * 把命中关键字的部分切成独立片段, 由模板渲染为强调标记.
 *
 * 分段而不是拼接 HTML: 内容来自来源站点, 拼 HTML 要额外考虑转义.
 */
const segments = computed<Segment[]>(() => {
  const needle = props.keyword.trim().toLowerCase();
  if (needle === "") {
    return [{ text: props.text, hit: false }];
  }

  const haystack = props.text.toLowerCase();
  const parts: Segment[] = [];
  let cursor = 0;
  for (;;) {
    const index = haystack.indexOf(needle, cursor);
    if (index === -1) {
      break;
    }
    if (index > cursor) {
      parts.push({ text: props.text.slice(cursor, index), hit: false });
    }
    parts.push({ text: props.text.slice(index, index + needle.length), hit: true });
    cursor = index + needle.length;
  }
  if (cursor < props.text.length) {
    parts.push({ text: props.text.slice(cursor), hit: false });
  }

  return parts.length === 0 ? [{ text: props.text, hit: false }] : parts;
});
</script>

<template>
  <template v-for="(part, index) in segments" :key="index">
    <mark v-if="part.hit" class="rounded bg-emerald-500/20 px-0.5 text-emerald-300">{{
      part.text
    }}</mark>
    <template v-else>{{ part.text }}</template>
  </template>
</template>
