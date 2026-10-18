// jumpTargetIndex のユニットテスト。ratingBar.test.ts と同じパターン。
import assert from "node:assert/strict";
import { test } from "node:test";
import { JUMP_PERCENTS, jumpTargetIndex } from "./jumpToPercent.ts";

test("JUMP_PERCENTSは0から100までの10刻み11個", () => {
  assert.deepEqual([...JUMP_PERCENTS], [0, 10, 20, 30, 40, 50, 60, 70, 80, 90, 100]);
});

test("0%は先頭、100%は最後の行", () => {
  assert.equal(jumpTargetIndex(250, 0), 0);
  assert.equal(jumpTargetIndex(250, 100), 249);
});

test("1件のときはどの割合でも0", () => {
  for (const p of JUMP_PERCENTS) assert.equal(jumpTargetIndex(1, p), 0);
});

test("11件では10%ごとに1行ずつ進み、100%は最後の行(10)に丸まる", () => {
  const actual = JUMP_PERCENTS.map((p) => jumpTargetIndex(11, p));
  assert.deepEqual(actual, [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
});

test("0件以下はnull", () => {
  assert.equal(jumpTargetIndex(0, 50), null);
  assert.equal(jumpTargetIndex(-1, 50), null);
});

test("割り切れない件数は切り捨てる", () => {
  assert.equal(jumpTargetIndex(7, 50), 3);
  assert.equal(jumpTargetIndex(7, 10), 0);
  assert.equal(jumpTargetIndex(7, 90), 6);
});

test("範囲外の割合は先頭/末尾に丸める", () => {
  assert.equal(jumpTargetIndex(10, -10), 0);
  assert.equal(jumpTargetIndex(10, 150), 9);
});
