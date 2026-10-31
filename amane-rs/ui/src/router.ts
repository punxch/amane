import { createRouter, createWebHistory } from "vue-router";

/**
 * 路由按页面拆分, 与后端能力一一对应.
 *
 * 列表与详情分成两个路由 (影片、媒体库), 详情因此有独立地址: 刷新、分享与返回都落在同一处.
 * 路由组件动态导入, 构建时各自成块.
 */
export const router = createRouter({
  history: createWebHistory(),
  routes: [
    {
      path: "/",
      name: "metadata",
      component: () => import("@/pages/MetadataListPage.vue"),
    },
    {
      path: "/metadata/:number",
      name: "metadata-detail",
      component: () => import("@/pages/MetadataDetailPage.vue"),
    },
    {
      path: "/libraries",
      name: "libraries",
      component: () => import("@/pages/LibraryListPage.vue"),
    },
    {
      path: "/libraries/:id",
      name: "library-detail",
      component: () => import("@/pages/LibraryDetailPage.vue"),
    },
    {
      path: "/tasks",
      name: "tasks",
      component: () => import("@/pages/TasksPage.vue"),
    },
    {
      path: "/schedules",
      name: "schedules",
      component: () => import("@/pages/SchedulesPage.vue"),
    },
    {
      path: "/settings",
      name: "settings",
      component: () => import("@/pages/SettingsPage.vue"),
    },
    { path: "/:pathMatch(.*)*", redirect: "/" },
  ],
});
