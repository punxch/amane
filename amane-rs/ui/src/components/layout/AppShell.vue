<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { RouterLink, RouterView, useRoute, useRouter } from "vue-router";

import ConnectionIndicator from "@/components/layout/ConnectionIndicator.vue";
import NavItem from "@/components/layout/NavItem.vue";
import ThemeToggle from "@/components/layout/ThemeToggle.vue";
import { NAV_GROUPS, SETTINGS_ITEM } from "@/components/layout/nav";
import HintedActionIcon from "@/components/common/HintedActionIcon.vue";
import Icon from "@/components/ui/Icon.vue";
import { APP_NAME, GITHUB_URL } from "@/lib/app";
import { connection, startHealthPolling, stopHealthPolling } from "@/stores/connection";
import { toggleNavbar, ui } from "@/stores/ui";

const route = useRoute();
const router = useRouter();

const mobileOpen = ref(false);
const keyword = ref("");

const version = computed(() => connection.version);

/**
 * 顶栏搜索: 交给片库页的 URL 参数, 由列表页自己解释.
 * 搜索词不留在顶栏里 —— 页面上的搜索框才是它唯一的输入位置.
 */
function submitSearch(): void {
  const q = keyword.value.trim();
  keyword.value = "";
  void router.push({ path: "/", query: q === "" ? {} : { q } });
}

// 跳转后收起窄屏抽屉; 同一路由内的搜索变化不触发.
watch(
  () => route.fullPath,
  () => {
    mobileOpen.value = false;
  },
);

onMounted(startHealthPolling);
onBeforeUnmount(stopHealthPolling);
</script>

<template>
  <div class="shell" :class="{ 'shell--collapsed': ui.navbarCollapsed }">
    <header class="shell__header">
      <div class="shell__header-inner">
        <HintedActionIcon
          label="打开导航"
          class="shell__burger"
          @click="mobileOpen = !mobileOpen"
        >
          <Icon name="menu" :size="18" />
        </HintedActionIcon>

        <HintedActionIcon
          :label="ui.navbarCollapsed ? '展开侧栏' : '收起侧栏'"
          class="shell__collapse"
          @click="toggleNavbar"
        >
          <Icon :name="ui.navbarCollapsed ? 'expand' : 'collapse'" :size="18" />
        </HintedActionIcon>

        <RouterLink to="/" class="shell__brand">
          <img src="/favicon.svg" width="22" height="22" alt="" aria-hidden="true" />
          <span class="shell__brand-name">{{ APP_NAME }}</span>
        </RouterLink>

        <a
          v-if="version !== null"
          class="shell__version"
          :href="`${GITHUB_URL}/releases`"
          target="_blank"
          rel="noreferrer"
          >v{{ version }}</a
        >

        <form class="shell__search" role="search" @submit.prevent="submitSearch">
          <Icon name="search" :size="16" class="shell__search-icon" />
          <input
            v-model="keyword"
            type="search"
            placeholder="搜索番号或标题"
            aria-label="搜索番号或标题"
          />
        </form>

        <div class="shell__actions">
          <ConnectionIndicator />
          <ThemeToggle />
          <a
            class="shell__github"
            :href="GITHUB_URL"
            target="_blank"
            rel="noreferrer"
            aria-label="GitHub"
          >
            <Icon name="github" :size="18" />
          </a>
        </div>
      </div>
    </header>

    <div
      v-if="mobileOpen"
      class="shell__scrim"
      aria-hidden="true"
      @click="mobileOpen = false"
    />

    <aside class="shell__navbar" :data-mobile-open="mobileOpen || undefined">
      <div class="shell__nav-scroll">
        <div v-for="group in NAV_GROUPS" :key="group.key" class="shell__group">
          <p class="shell__group-label">{{ group.label }}</p>
          <NavItem v-for="item in group.items" :key="item.to" :item="item" />
        </div>
      </div>

      <div class="shell__nav-footer">
        <NavItem :item="SETTINGS_ITEM" />
        <!-- 窄屏顶栏容纳不下版本与外链, 二者在侧栏底部给出. -->
        <div class="shell__nav-meta">
          <a
            v-if="version !== null"
            class="shell__meta-version"
            :href="`${GITHUB_URL}/releases`"
            target="_blank"
            rel="noreferrer"
            >v{{ version }}</a
          >
          <a
            class="shell__meta-github"
            :href="GITHUB_URL"
            target="_blank"
            rel="noreferrer"
            aria-label="GitHub"
          >
            <Icon name="github" :size="16" />
          </a>
        </div>
      </div>
    </aside>

    <main class="shell__main">
      <RouterView />
    </main>
  </div>
</template>

<style scoped>
.shell {
  display: grid;
  grid-template-columns: var(--amane-navbar-width) minmax(0, 1fr);
  grid-template-rows: var(--amane-header-height) minmax(0, 1fr);
  height: var(--amane-vh);
  overflow: hidden;
  background: var(--amane-body);
}

.shell--collapsed {
  grid-template-columns: 0 minmax(0, 1fr);
}

.shell__header {
  grid-column: 1 / -1;
  border-bottom: 1px solid var(--amane-line);
  background: var(--amane-body);
}

.shell__header-inner {
  display: flex;
  align-items: center;
  gap: 6px;
  height: 100%;
  padding: 0 16px;
}

.shell__burger {
  display: inline-flex;
}

.shell__collapse {
  display: none;
}

.shell__brand {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  margin-left: 4px;
  white-space: nowrap;
}

.shell__brand-name {
  color: var(--amane-bright);
  font-size: 18px;
  font-weight: 600;
  letter-spacing: -0.01em;
}

.shell__version {
  margin-left: 2px;
  color: var(--amane-dimmed);
  font-family: var(--font-mono);
  font-size: 12px;
  white-space: nowrap;
}

.shell__version:hover {
  color: var(--amane-fg);
}

.shell__search {
  position: relative;
  display: none;
  flex: 1;
  max-width: 480px;
  min-width: 0;
  margin-left: 24px;
}

.shell__search-icon {
  position: absolute;
  top: 50%;
  left: 11px;
  transform: translateY(-50%);
  color: var(--amane-dimmed);
  pointer-events: none;
}

.shell__search input {
  width: 100%;
  height: 34px;
  padding: 0 12px 0 34px;
  border: 1px solid var(--amane-line);
  border-radius: var(--amane-radius);
  background: var(--amane-body);
  color: var(--amane-fg);
  font: inherit;
  font-size: 14px;
  outline: none;
  transition:
    border-color 100ms ease,
    box-shadow 100ms ease;
}

.shell__search input::placeholder {
  color: var(--amane-subtle);
}

.shell__search input:focus {
  border-color: var(--amane-brand);
  box-shadow: 0 0 0 2px var(--amane-brand-soft);
}

.shell__actions {
  display: flex;
  align-items: center;
  gap: 2px;
  margin-left: auto;
}

.shell__github {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 34px;
  height: 34px;
  border-radius: var(--amane-radius);
  color: var(--amane-dimmed);
}

.shell__github:hover {
  background: var(--amane-surface-2);
  color: var(--amane-bright);
}

.shell__navbar {
  grid-column: 1;
  grid-row: 2;
  display: flex;
  flex-direction: column;
  min-height: 0;
  border-right: 1px solid var(--amane-line);
  background: var(--amane-body);
  overflow: hidden;
}

.shell__nav-scroll {
  flex: 1;
  min-height: 0;
  padding: 12px;
  overflow-y: auto;
  overscroll-behavior: contain;
}

.shell__group {
  margin-bottom: 16px;
}

.shell__group-label {
  margin: 0 0 4px;
  padding: 0 12px;
  color: var(--amane-dimmed);
  font-size: 12px;
  font-weight: 700;
  letter-spacing: 0.5px;
  text-transform: uppercase;
}

.shell__nav-footer {
  padding: 12px;
  border-top: 1px solid var(--amane-line);
}

.shell__nav-meta {
  display: none;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 8px 12px 0;
}

.shell__meta-version {
  color: var(--amane-dimmed);
  font-family: var(--font-mono);
  font-size: 12px;
}

.shell__meta-github {
  display: inline-flex;
  color: var(--amane-dimmed);
}

.shell__scrim {
  display: none;
}

.shell__main {
  grid-column: 2;
  grid-row: 2;
  min-width: 0;
  padding: 16px;
  overflow: auto;
}

@media (min-width: 48em) {
  .shell__burger {
    display: none;
  }

  .shell__collapse {
    display: inline-flex;
  }

  .shell__search {
    display: flex;
  }
}

@media (max-width: 47.99em) {
  .shell {
    grid-template-columns: 0 minmax(0, 1fr);
  }

  /* 窄屏顶栏只保留品牌与图标动作; 版本与外链在侧栏底部给出. */
  .shell__version,
  .shell__github {
    display: none;
  }

  .shell__nav-meta {
    display: flex;
  }

  .shell__navbar {
    position: fixed;
    top: var(--amane-header-height);
    bottom: 0;
    left: 0;
    z-index: 50;
    width: 280px;
    border-right: 1px solid var(--amane-line);
    transform: translateX(-100%);
    transition: transform 150ms ease;
  }

  .shell__navbar[data-mobile-open="true"] {
    transform: translateX(0);
  }

  .shell__scrim {
    position: fixed;
    inset: var(--amane-header-height) 0 0;
    z-index: 40;
    display: block;
    background: var(--amane-overlay);
  }

  .shell__main {
    padding: 12px;
  }
}
</style>
