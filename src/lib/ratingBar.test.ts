// toggleRating のユニットテスト。
import assert from "node:assert/strict";
import { test } from "node:test";
import { RATING_FILTER_OPTIONS, ratingLabel, toggleRating } from "./ratingBar.ts";

test("無効な評価を切り替えると追加される", () => {
  assert.deepEqual(toggleRating([], 3), [3]);
});

test("有効な評価をもう一度切り替えると外れる", () => {
  assert.deepEqual(toggleRating([3], 3), []);
  assert.deepEqual(toggleRating([1, 3, 5], 3), [1, 5]);
});

test("複数の評価を同時に有効にでき、結果は昇順にそろう", () => {
  assert.deepEqual(toggleRating([2], 1), [1, 2]);
  assert.deepEqual(toggleRating([5, 1], 3), [1, 3, 5]);
});

test("入力の配列は変更せず、新しい配列を返す", () => {
  const input = [1, 2];
  const out = toggleRating(input, 3);
  assert.deepEqual(input, [1, 2]);
  assert.notEqual(out, input);
  const removed = toggleRating(input, 1);
  assert.deepEqual(input, [1, 2]);
  assert.deepEqual(removed, [2]);
});

test("ratingLabel は★をN個並べ、左ペインの選択肢と同じ表記になる", () => {
  assert.equal(ratingLabel(3), "★★★");
  for (const opt of RATING_FILTER_OPTIONS) assert.equal(opt.label, ratingLabel(opt.value));
});
