// computeWbImportSteps / showsRowProgress のユニットテスト。
// src/lib/ratingBar.test.ts と同じパターン(package.jsonのtest:unit参照)。
import assert from "node:assert/strict";
import { test } from "node:test";
import {
  computeWbImportSteps,
  isWbImportStageId,
  showsRowProgress,
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

test("進捗バーは動画の登録の段階だけ", () => {
  assert.equal(showsRowProgress("registering"), true);
  assert.equal(showsRowProgress("reading"), false);
  assert.equal(showsRowProgress("linking"), false);
  assert.equal(showsRowProgress(null), false);
});

test("段階名の判定は未知の文字列を弾く", () => {
  assert.equal(isWbImportStageId("linking"), true);
  assert.equal(isWbImportStageId("rebuild"), false);
});
