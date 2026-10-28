import { createApp } from "vue";

import App from "@/App.vue";
import { captureToken } from "@/api/client";
import { router } from "@/router";
import "@/style.css";

captureToken();
createApp(App).use(router).mount("#app");
