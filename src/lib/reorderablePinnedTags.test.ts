// moveUp/moveDown のユニットテスト。
// src/lib/sidebarResize.test.ts と同じパターン(Node組み込みtest runnerを
// `node --experimental-strip-types --test`で実行、package.jsonのtest:unit
// 参照)。
import assert from "node:assert/strict";
import { test } from "node:test";
import { moveDown, moveUp } from "./reorderablePinnedTags.ts";

test("moveUp: 中間の要素を1つ前と入れ替える", () => {
  assert.deepEqual(moveUp([1, 2, 3], 1), [2, 1, 3]);
});

test("moveUp: 先頭(index 0)ではno-opで同じ内容を返す", () => {
  assert.deepEqual(moveUp([1, 2, 3], 0), [1, 2, 3]);
});

test("moveUp: 範囲外のindexではno-op", () => {
  assert.deepEqual(moveUp([1, 2, 3], 3), [1, 2, 3]);
  assert.deepEqual(moveUp([1, 2, 3], -1), [1, 2, 3]);
});

test("moveDown: 中間の要素を1つ後ろと入れ替える", () => {
  assert.deepEqual(moveDown([1, 2, 3], 1), [1, 3, 2]);
});

test("moveDown: 末尾ではno-opで同じ内容を返す", () => {
  assert.deepEqual(moveDown([1, 2, 3], 2), [1, 2, 3]);
});

test("moveDown: 範囲外のindexではno-op", () => {
  assert.deepEqual(moveDown([1, 2, 3], 5), [1, 2, 3]);
});

test("moveUp/moveDown: string配列でも入れ替えられる", () => {
  assert.deepEqual(moveUp(["a", "b", "c"], 2), ["a", "c", "b"]);
  assert.deepEqual(moveDown(["a", "b", "c"], 0), ["b", "a", "c"]);
});

test("moveUp/moveDown: string配列の端ではno-op", () => {
  assert.deepEqual(moveUp(["a", "b"], 0), ["a", "b"]);
  assert.deepEqual(moveDown(["a", "b"], 1), ["a", "b"]);
});
