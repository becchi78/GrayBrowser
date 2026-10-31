// computeWbImportSteps / formatStepCount / progressBarValue のユニットテスト。
// src/lib/ratingBar.test.ts と同じパターン(package.jsonのtest:unit参照)。
import assert from "node:assert/strict";
import { test } from "node:test";
import {
  computeWbImportSteps,
  formatStepCount,
  isWbImportStageId,
  progressBarValue,
} from "./wbImportSteps.ts";
import type { WbImportStageId } from "../types.ts";

function statuses(stage: WbImportStageId | null, outcome: "complete" | "failed" | null) {
  return computeWbImportSteps(stage, outcome).map((s) => s.status);
}

test("段階はUUID再構築を含まない3つで、順序が固定", () => {
  const steps = computeWbImportSteps(null, null);
  assert.deepEqual(
    steps.map((s) => s.key),
    ["reading", "registering", "linking"],
  );
  assert.ok(steps.every((s) => !s.label.includes("UUID")));
});

test("段階イベント前は全て未着手", () => {
  assert.deepEqual(statuses(null, null), ["pending", "pending", "pending"]);
});

test("読み込み中は読み込みだけが実行中", () => {
  assert.deepEqual(statuses("reading", null), ["active", "pending", "pending"]);
});

test("登録中は読み込みが完了し登録だけが実行中", () => {
  assert.deepEqual(statuses("registering", null), ["done", "active", "pending"]);
});

test("紐付け中は登録までが完了し紐付けだけが実行中", () => {
  assert.deepEqual(statuses("linking", null), ["done", "done", "active"]);
});

test("どの段階でも実行中は高々1つ", () => {
  for (const stage of [null, "reading", "registering", "linking"] as const) {
    for (const outcome of [null, "complete", "failed"] as const) {
      const active = statuses(stage, outcome).filter((s) => s === "active").length;
      assert.ok(active <= 1, `${stage}/${outcome}`);
    }
  }
});

test("完了後は、最後の段階が何であっても全て完了", () => {
  assert.deepEqual(statuses("linking", "complete"), ["done", "done", "done"]);
  assert.deepEqual(statuses(null, "complete"), ["done", "done", "done"]);
});

test("失敗後は、失敗した段階とそれ以降を未着手のままにする", () => {
  assert.deepEqual(statuses("reading", "failed"), ["pending", "pending", "pending"]);
  assert.deepEqual(statuses("registering", "failed"), ["done", "pending", "pending"]);
});

test("未着手の段階は件数を出さない", () => {
  assert.equal(formatStepCount("pending", { processed: 3, total: 10 }), "");
  assert.equal(formatStepCount("pending", undefined), "");
});

test("実行中で件数がまだ届いていなければ件数を出さない", () => {
  assert.equal(formatStepCount("active", undefined), "");
});

test("実行中は、分母ありなら「x / y件」、分母なしなら「x件」", () => {
  assert.equal(formatStepCount("active", { processed: 12, total: 50 }), "12 / 50件");
  assert.equal(formatStepCount("active", { processed: 12, total: null }), "12件");
});

test("完了は最終件数。分母があればそれ、無ければ処理済み件数", () => {
  assert.equal(formatStepCount("done", { processed: 49, total: 50 }), "50件");
  assert.equal(formatStepCount("done", { processed: 7, total: null }), "7件");
  assert.equal(formatStepCount("done", undefined), "");
});

test("件数0でも崩れない", () => {
  assert.equal(formatStepCount("active", { processed: 0, total: 0 }), "0 / 0件");
  assert.equal(formatStepCount("active", { processed: 0, total: null }), "0件");
  assert.equal(formatStepCount("done", { processed: 0, total: 0 }), "0件");
});

test("件数は3桁区切りにする", () => {
  assert.equal(formatStepCount("active", { processed: 1200, total: 5000 }), "1,200 / 5,000件");
  assert.equal(formatStepCount("done", { processed: 1234567, total: null }), "1,234,567件");
});

test("進捗バーは、実行中の段階に分母があるときだけ出す", () => {
  const counts = {
    reading: { processed: 5, total: 5 },
    registering: { processed: 20, total: 50 },
    linking: { processed: 1, total: null },
  };
  assert.deepEqual(progressBarValue("registering", counts), { value: 20, max: 50 });
  assert.deepEqual(progressBarValue("reading", counts), { value: 5, max: 5 });
  assert.equal(progressBarValue("linking", counts), null);
  assert.equal(progressBarValue(null, counts), null);
  assert.equal(progressBarValue("registering", {}), null);
});

test("分母が0の段階には進捗バーを出さない", () => {
  assert.equal(progressBarValue("linking", { linking: { processed: 0, total: 0 } }), null);
});

test("進捗バーの値は分母を超えない", () => {
  assert.deepEqual(progressBarValue("registering", { registering: { processed: 9, total: 5 } }), {
    value: 5,
    max: 5,
  });
});

test("段階名の判定は未知の文字列を弾く", () => {
  assert.equal(isWbImportStageId("linking"), true);
  assert.equal(isWbImportStageId("rebuild"), false);
});
