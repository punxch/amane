<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from "vue";
import { RouterLink, RouterView } from "vue-router";

import { api, type HealthResponse } from "@/api/client";

/** 健康探测间隔. 只影响顶部状态, 不阻塞任何页面. */
const HEALTH_POLL_MS = 15000;

const health = ref<HealthResponse | null>(null);
const unreachable = ref(false);
let timer: number | undefined;

async function probe(): Promise<void> {
  try {
    health.value = await api.health();
    unreachable.value = false;
  } catch {
    health.value = null;
    unreachable.value = true;
  }
}

onMounted(() => {
  void probe();
  timer = window.setInterval(() => void probe(), HEALTH_POLL_MS);
});

onBeforeUnmount(() => {
  if (timer !== undefined) {
    window.clearInterval(timer);
  }
});
</script>

<template>
  <div class="min-h-full">
    <header class="border-b border-neutral-800/80 px-8 py-4">
      <div class="flex items-baseline gap-8">
        <div class="flex items-baseline gap-3">
          <span class="text-lg font-semibold tracking-tight text-neutral-100">Amane</span>
          <span class="text-xs text-neutral-500">影片元数据服务</span>
        </div>
        <nav class="flex gap-1 text-sm">
          <RouterLink
            to="/"
            class="rounded-md px-3 py-1 text-neutral-400 hover:text-neutral-200"
            exact-active-class="bg-neutral-900 text-neutral-100"
          >
            影片
          </RouterLink>
          <RouterLink
            to="/libraries"
            class="rounded-md px-3 py-1 text-neutral-400 hover:text-neutral-200"
            exact-active-class="bg-neutral-900 text-neutral-100"
          >
            媒体库
          </RouterLink>
          <RouterLink
            to="/settings"
            class="rounded-md px-3 py-1 text-neutral-400 hover:text-neutral-200"
            exact-active-class="bg-neutral-900 text-neutral-100"
          >
            设置
          </RouterLink>
          <RouterLink
            to="/schedules"
            class="rounded-md px-3 py-1 text-neutral-400 hover:text-neutral-200"
            exact-active-class="bg-neutral-900 text-neutral-100"
          >
            定时任务
          </RouterLink>
          <RouterLink
            to="/tasks"
            class="rounded-md px-3 py-1 text-neutral-400 hover:text-neutral-200"
            exact-active-class="bg-neutral-900 text-neutral-100"
          >
            任务
          </RouterLink>
        </nav>

        <div class="ml-auto flex items-center gap-2 text-xs">
          <template v-if="unreachable">
            <span class="h-1.5 w-1.5 rounded-full bg-red-500"></span>
            <span class="text-red-400">无法连接后端服务</span>
          </template>
          <template v-else-if="health !== null">
            <span class="h-1.5 w-1.5 rounded-full bg-emerald-500"></span>
            <span class="text-neutral-500">
              {{ health.status === "ok" ? "就绪" : health.status }}
            </span>
            <span class="font-mono text-neutral-600">v{{ health.version }}</span>
          </template>
          <span v-else class="text-neutral-600">检查中…</span>
        </div>
      </div>
    </header>

    <RouterView />
  </div>
</template>
