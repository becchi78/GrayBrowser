// errorText のユニットテスト。
import assert from "node:assert/strict";
import { test } from "node:test";
import { errorText } from "./errorText.ts";

test("errorText は文字列をそのまま返す", () => {
  assert.equal(errorText("boom"), "boom");
  assert.equal(errorText(""), "");
});

test("errorText は Error の message を返す", () => {
  assert.equal(errorText(new Error("bad")), "bad");
  assert.equal(errorText(new TypeError("type")), "type");
});

test("errorText はそれ以外の値を String で文字列にする", () => {
  assert.equal(errorText(42), "42");
  assert.equal(errorText({ code: 1 }), "[object Object]");
  assert.equal(errorText(null), "null");
  assert.equal(errorText(undefined), "undefined");
});
