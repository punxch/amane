import { createApp } from "vue";

import App from "@/App.vue";
import { captureToken } from "@/api/client";
import { router } from "@/router";
import "@/stores/ui";
import "@/style.css";

/**
 * 视口高度: 默认 vh, 支持 dvh 的引擎换成 dvh.
 *
 * 判定用实测布局而不是 `CSS.supports`: 厂商自带的 WebView 可能在特性查询里声称支持 dvh 却算不出高度,
 * 换过去会让依赖它的声明失效.
 */
const viewportProbe = document.createElement("div");
viewportProbe.style.cssText = "position:absolute;height:100dvh;width:0;visibility:hidden";
document.body.appendChild(viewportProbe);
if (viewportProbe.offsetHeight > 0) {
  document.documentElement.style.setProperty("--amane-vh", "100dvh");
}
viewportProbe.remove();

captureToken();
createApp(App).use(router).mount("#app");
