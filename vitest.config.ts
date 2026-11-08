import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

// vite.config.ts は Tauri 用の server 設定を持つ async 関数設定で mergeConfig
// しにくく、テストには不要なため、plugin だけを共有する独立の設定にしている。
export default defineConfig({
  plugins: [react()],
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.tsx"],
    setupFiles: ["src/testing/setup.ts"],
    globals: false,
    // 見た目の検証はしないので CSS の変換を省いて高速化する。
    css: false,
    clearMocks: true,
    restoreMocks: true,
  },
});
