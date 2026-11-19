import "./assets/fonts/fonts.css";
import "./styles/icons.css";
import "./styles/ui.css";
import { createApp } from "vue";
import App from "./App.vue";
import router from "./router";
import { vTruncate } from "./directives/truncateTooltip";
import { vTooltip } from "./directives/tooltip";
import { installDevCapture } from "./composables/useDevTools";

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
