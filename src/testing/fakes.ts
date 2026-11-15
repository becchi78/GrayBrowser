import { vi } from "vitest";
import type * as api from "../api";

/** 更新の確認結果(Update)のフェイク。downloadAndInstall の挙動をテストごとに差し込む。 */
export function fakeUpdate(downloadAndInstall: () => Promise<void>) {
  return {
    version: "2.0.0",
    currentVersion: "1.0.0",
    body: "ノート",
    close: vi.fn().mockResolvedValue(undefined),
    downloadAndInstall: vi.fn(downloadAndInstall),
  } as unknown as NonNullable<Awaited<ReturnType<typeof api.checkForUpdate>>>;
}
