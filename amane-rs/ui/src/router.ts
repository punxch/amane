import { createRouter, createWebHistory } from "vue-router";

import MetadataPage from "@/pages/MetadataPage.vue";
import SchedulesPage from "@/pages/SchedulesPage.vue";
import TasksPage from "@/pages/TasksPage.vue";

export const router = createRouter({
  history: createWebHistory(),
  routes: [
    { path: "/", name: "metadata", component: MetadataPage },
    { path: "/tasks", name: "tasks", component: TasksPage },
    { path: "/schedules", name: "schedules", component: SchedulesPage },
  ],
});
