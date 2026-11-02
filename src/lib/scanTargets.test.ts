// countScanTargets / rescanState のユニットテスト。
// src/lib/reachability.test.ts と同じパターン(package.jsonのtest:unit参照)。
import assert from "node:assert/strict";
import { test } from "node:test";
import { countScanTargets, rescanState } from "./scanTargets.ts";

test("更新チェックを外したフォルダは、対象数に数えない", () => {
  assert.equal(countScanTargets(["A", "B", "C"], ["B"]), 2);
});

test("外したフォルダがなければ、全フォルダが対象", () => {
  assert.equal(countScanTargets(["A", "B"], []), 2);
});

test("登録にないパスが外し済みリストに残っていても、数は変わらない", () => {
  assert.equal(countScanTargets(["A"], ["Z"]), 1);
});

test("フォルダが0件なら、無効(理由: フォルダなし)", () => {
  assert.deepEqual(rescanState([], []), { enabled: false, reason: "no-folders" });
});

test("全フォルダが更新チェックオフなら、無効(理由: 全オフ)", () => {
  assert.deepEqual(rescanState(["A", "B"], ["A", "B"]), { enabled: false, reason: "all-check-off" });
});

test("1つでも更新チェックが入っていれば、有効", () => {
  assert.deepEqual(rescanState(["A", "B"], ["A"]), { enabled: true });
});
