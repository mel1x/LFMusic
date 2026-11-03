import "./assets/fonts/fonts.css";
import { createApp } from "vue";
import App from "./App.vue";
import router from "./router";
import { vTruncate } from "./directives/truncateTooltip";
import { vTooltip } from "./directives/tooltip";
import { installDevCapture } from "./composables/useDevTools";

// Records startup invokes; removed again once settings load if dev mode is off.
try {
    installDevCapture();
} catch (e) {
    console.warn("Dev capture failed:", e);
}

createApp(App)
    .use(router)
    .directive("truncate", vTruncate)
    .directive("tooltip", vTooltip)
    .mount("#app");
