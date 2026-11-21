import { vi } from "vitest";
import type * as api from "../api";

type UpdateResult = NonNullable<Awaited<ReturnType<typeof api.checkForUpdate>>>;

/**
 * 更新の確認結果(Update)のフェイク。downloadAndInstall の挙動をテストごとに差し込む。
 * 受け取ったコールバックに進捗イベントを流す処理も書ける。body は省略時「ノート」。
 */
export function fakeUpdate(
  downloadAndInstall: (
    onEvent: Parameters<UpdateResult["downloadAndInstall"]>[0] & {},
  ) => Promise<void>,
  { body = "ノート", version = "2.0.0" }: { body?: string | null; version?: string } = {},
) {
  return {
    version,
    currentVersion: "1.0.0",
    body,
    close: vi.fn().mockResolvedValue(undefined),
    downloadAndInstall: vi.fn(downloadAndInstall),
  } as unknown as UpdateResult;
}
