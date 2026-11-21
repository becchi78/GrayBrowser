import assert from "node:assert/strict";
import { test } from "node:test";
import { nextFocusIndex } from "./menuFocus.ts";

test("next / prev は端で反対側へ回る", () => {
  assert.equal(nextFocusIndex(2, 3, "next"), 0);
  assert.equal(nextFocusIndex(0, 3, "prev"), 2);
  assert.equal(nextFocusIndex(0, 3, "next"), 1);
  assert.equal(nextFocusIndex(1, 3, "prev"), 0);
});

test("どの項目にもフォーカスが無い(-1)とき、next は先頭、prev は末尾", () => {
  assert.equal(nextFocusIndex(-1, 3, "next"), 0);
  assert.equal(nextFocusIndex(-1, 3, "prev"), 2);
});

test("first / last は現在位置によらない", () => {
  assert.equal(nextFocusIndex(1, 4, "first"), 0);
  assert.equal(nextFocusIndex(1, 4, "last"), 3);
});

test("項目が無ければ -1", () => {
  assert.equal(nextFocusIndex(-1, 0, "next"), -1);
});
