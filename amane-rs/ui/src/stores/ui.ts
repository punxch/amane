/**
 * 界面偏好.
 *
 * 与原项目 `web/src/stores/ui.ts` 的持久化键同形: 主题、侧栏折叠、分页大小都存在本地,
 * 刷新后保持. 列表的筛选与页码不在这里 —— 它们属于 URL, 侧栏入口带过来的默认值才落在这里.
 */
import { reactive, watch } from "vue";

export type Theme = "light" | "dark" | "auto";

/** 可选分页大小; 页面从这个元组取值, 不接受任意数字. */
export const PAGE_SIZE_OPTIONS = [24, 50, 100] as const;
export type PageSize = (typeof PAGE_SIZE_OPTIONS)[number];

export interface UIState {
  theme: Theme;
  navbarCollapsed: boolean;
  pageSize: PageSize;
}

const STORAGE_KEY = "amane-ui";

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function isTheme(value: unknown): value is Theme {
  return value === "light" || value === "dark" || value === "auto";
}

function isPageSize(value: unknown): value is PageSize {
  return typeof value === "number" && PAGE_SIZE_OPTIONS.some((size) => size === value);
}

/** 读取本地偏好; 任何一项不合法都退回默认值, 不整份丢弃. */
function readStored(): Partial<UIState> {
  const raw = window.localStorage.getItem(STORAGE_KEY);
  if (raw === null) {
    return {};
  }

  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return {};
  }
  if (!isRecord(parsed)) {
    return {};
  }

  const stored: Partial<UIState> = {};
  if (isTheme(parsed.theme)) {
    stored.theme = parsed.theme;
  }
  if (typeof parsed.navbarCollapsed === "boolean") {
    stored.navbarCollapsed = parsed.navbarCollapsed;
  }
  if (isPageSize(parsed.pageSize)) {
    stored.pageSize = parsed.pageSize;
  }
  return stored;
}

export const ui = reactive<UIState>({
  theme: "dark",
  navbarCollapsed: false,
  pageSize: 50,
  ...readStored(),
});

const darkScheme = window.matchMedia("(prefers-color-scheme: dark)");

/** 主题的实际取值: `auto` 跟随系统, 其余原样. */
export function resolvedTheme(): "light" | "dark" {
  if (ui.theme === "auto") {
    return darkScheme.matches ? "dark" : "light";
  }
  return ui.theme;
}

/** 写入根元素的 `data-theme`; 颜色令牌由 style.css 按它切换. */
export function applyTheme(): void {
  document.documentElement.dataset.theme = resolvedTheme();
}

/** 主题按 浅色 → 深色 → 跟随系统 轮换, 与顶栏按钮的说明一致. */
export function cycleTheme(): void {
  ui.theme = ui.theme === "light" ? "dark" : ui.theme === "dark" ? "auto" : "light";
  applyTheme();
}

export function toggleNavbar(): void {
  ui.navbarCollapsed = !ui.navbarCollapsed;
}

export function setPageSize(size: PageSize): void {
  ui.pageSize = size;
}

darkScheme.addEventListener("change", () => {
  if (ui.theme === "auto") {
    applyTheme();
  }
});

watch(
  ui,
  (state) => {
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(state));
  },
  { deep: true },
);

applyTheme();
