// withoutKey / withoutItem のユニットテスト。
import assert from "node:assert/strict";
import { test } from "node:test";
import { withoutItem, withoutKey } from "./withoutKey.ts";

test("withoutKey は指定キーを除いた新しいオブジェクトを返す", () => {
  assert.deepEqual(withoutKey({ a: "x", b: "y" }, "a"), { b: "y" });
});

test("withoutKey は元のオブジェクトを変更しない", () => {
  const src = { a: "x", b: "y" };
  const out = withoutKey(src, "a");
  assert.notEqual(out, src);
  assert.deepEqual(src, { a: "x", b: "y" });
});

test("withoutKey はキーが無くても壊れず、同じ中身のコピーを返す", () => {
  const src = { a: "x" };
  const out = withoutKey(src, "zzz");
  assert.deepEqual(out, { a: "x" });
  assert.notEqual(out, src);
});

test("withoutKey は空のオブジェクトでも壊れない", () => {
  assert.deepEqual(withoutKey({}, "a"), {});
});

test("withoutItem は指定要素を除いた新しい Set を返す", () => {
  assert.deepEqual([...withoutItem(new Set(["a", "b"]), "a")], ["b"]);
});

test("withoutItem は元の Set を変更しない", () => {
  const src = new Set(["a", "b"]);
  const out = withoutItem(src, "a");
  assert.notEqual(out, src);
  assert.deepEqual([...src], ["a", "b"]);
});

test("withoutItem は要素が無くても壊れず、同じ中身のコピーを返す", () => {
  const src = new Set(["a"]);
  const out = withoutItem(src, "zzz");
  assert.deepEqual([...out], ["a"]);
  assert.notEqual(out, src);
});
