import { createApp } from "vue";
import App from "./App.vue";
import router from "./router";
import { vTruncate } from "./directives/truncateTooltip";
import { vTooltip } from "./directives/tooltip";

createApp(App)
    .use(router)
    .directive("truncate", vTruncate)
    .directive("tooltip", vTooltip)
    .mount("#app");
