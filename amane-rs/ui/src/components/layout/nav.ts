/**
 * 侧栏结构.
 *
 * 分组与原项目一致: 浏览 / 管理 / 运行, 设置单独固定在底部. 只登记本实现已有页面的入口 ——
 * 空入口会把人带到 404.
 */
import type { IconName } from "@/components/ui/Icon.vue";

export interface NavItem {
  to: string;
  label: string;
  icon: IconName;
  /** 只在该路径本身高亮; 默认连同子路径. */
  exact?: boolean;
}

export interface NavGroup {
  key: string;
  label: string;
  items: NavItem[];
}

export const NAV_GROUPS: readonly NavGroup[] = [
  {
    key: "browse",
    label: "浏览",
    items: [{ to: "/", label: "片库", icon: "movie", exact: true }],
  },
  {
    key: "manage",
    label: "管理",
    items: [{ to: "/libraries", label: "媒体库", icon: "folder" }],
  },
  {
    key: "ops",
    label: "运行",
    items: [
      { to: "/tasks", label: "任务", icon: "list" },
      { to: "/schedules", label: "定时任务", icon: "clock" },
    ],
  },
];

export const SETTINGS_ITEM: NavItem = { to: "/settings", label: "设置", icon: "settings" };

/**
 * 高亮判定.
 *
 * `/` 同时覆盖影片详情 (`/metadata/…`), 否则从详情页回到列表时侧栏没有任何亮项.
 */
export function isNavActive(path: string, item: NavItem): boolean {
  if (item.to === "/") {
    return path === "/" || path.startsWith("/metadata/");
  }
  if (item.exact === true) {
    return path === item.to;
  }
  return path === item.to || path.startsWith(`${item.to}/`);
}
