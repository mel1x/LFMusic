import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";
import { fileURLToPath } from "node:url";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

export default defineConfig(async () => ({
  plugins: [vue()],

  resolve: {
    alias: [
      // App invokes go through a wrapper that dev mode can hook (src/utils/tauriCore.ts).
      {
        find: /^@tauri-apps\/api\/core$/,
        replacement: fileURLToPath(new URL("./src/utils/tauriCore.ts", import.meta.url)),
      },
    ],
  },

  build: {
    // WebView2 / WKWebView are modern engines: no legacy transpiling.
    target:
      process.env.TAURI_ENV_PLATFORM === "windows" || process.env.TAURI_ENV_PLATFORM === "android"
        ? "chrome105"
        : "safari13",
    minify: !process.env.TAURI_ENV_DEBUG,
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
  },

  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: true,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
}));
