<script setup lang="ts">
import { RouterLink, RouterView } from "vue-router";
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
      </div>
    </header>

    <RouterView />
  </div>
</template>
