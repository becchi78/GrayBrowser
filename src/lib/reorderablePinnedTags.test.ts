// moveUp/moveDown/removeAt/addIfAbsent のユニットテスト。
// src/lib/sidebarResize.test.ts と同じパターン(Node組み込みtest runnerを
// `node --experimental-strip-types --test`で実行、package.jsonのtest:unit
// 参照)。
import assert from "node:assert/strict";
import { test } from "node:test";
import { addIfAbsent, moveDown, moveUp, removeAt } from "./reorderablePinnedTags.ts";

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

test("removeAt: 指定したindexの要素だけを取り除く", () => {
  assert.deepEqual(removeAt([1, 2, 3], 1), [1, 3]);
});

test("removeAt: 範囲外のindexではno-op", () => {
  assert.deepEqual(removeAt([1, 2, 3], 3), [1, 2, 3]);
  assert.deepEqual(removeAt([1, 2, 3], -1), [1, 2, 3]);
});

test("removeAt: 空配列に対してもno-op", () => {
  assert.deepEqual(removeAt([], 0), []);
});

test("addIfAbsent: 未追加のidは末尾に追加される", () => {
  assert.deepEqual(addIfAbsent([1, 2], 3), [1, 2, 3]);
});

test("addIfAbsent: 既に含まれるidは重複追加されない", () => {
  assert.deepEqual(addIfAbsent([1, 2], 2), [1, 2]);
});

test("addIfAbsent: 空配列への追加", () => {
  assert.deepEqual(addIfAbsent([], 1), [1]);
});
