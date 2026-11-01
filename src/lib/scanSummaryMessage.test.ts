import assert from "node:assert/strict";
import { describe, it } from "node:test";
import type { ScanSummary } from "../types";
import { buildScanSummaryMessage } from "./scanSummaryMessage.ts";

function summary(unreachable: string[]): ScanSummary {
  return {
    scanned: 5,
    registered: 1,
    reconciled: 2,
    unchanged: 3,
    skipped: 4,
    went_offline: 6,
    reactivated: 7,
    collisions: 0,
    unreachable_folders: unreachable,
  };
}

describe("buildScanSummaryMessage", () => {
  it("omits the unreachable part when no folder was skipped", () => {
    assert.equal(
      buildScanSummaryMessage(summary([])),
      "検出: 5 / 登録: 1 / 更新: 2 / 変化なし: 3 / スキップ: 4 / オフライン化: 6 / 復帰: 7",
    );
  });

  it("appends the number of skipped folders when there are any", () => {
    assert.match(
      buildScanSummaryMessage(summary(["W:/a", "W:/b"])),
      / \/ 到達できないため飛ばしたフォルダ: 2 件$/,
    );
  });
});
