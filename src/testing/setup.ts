import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { afterEach, vi } from "vitest";

afterEach(() => {
  cleanup();
});

// 本物の IPC・更新サーバーにテストが意図せず触れないよう、Tauri の入口を
// 「呼ばれたら失敗する」フェイクに差し替える。必要なテストだけが
// vi.mocked(...).mockResolvedValue(...) などで明示的に上書きする。
function unmocked(name: string) {
  return (...args: unknown[]): never => {
    throw new Error(
      `未モックの Tauri 呼び出し: ${name}(${args.map((a) => JSON.stringify(a)).join(", ")})`,
    );
  };
}

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(unmocked("invoke")),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(unmocked("listen")),
}));
vi.mock("@tauri-apps/api/app", () => ({
  getVersion: vi.fn(unmocked("getVersion")),
}));
vi.mock("@tauri-apps/plugin-updater", () => ({
  Update: class {
    constructor() {
      unmocked("new Update")();
    }
  },
}));
