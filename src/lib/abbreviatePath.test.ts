import assert from "node:assert/strict";
import { test } from "node:test";
import { abbreviatePath } from "./abbreviatePath.ts";

test("上限以下ならそのまま返す", () => {
  assert.equal(abbreviatePath("C:\\a\\b.wb", 20), "C:\\a\\b.wb");
  assert.equal(abbreviatePath("12345", 5), "12345");
  assert.equal(abbreviatePath("", 5), "");
});

test("上限を超えたら先頭と末尾を残して中間を省略し、長さはちょうど上限になる", () => {
  const path = "C:\\Users\\someone\\Documents\\WhiteBrowser\\thumb\\200x150x6x1";
  const out = abbreviatePath(path, 30);
  assert.equal(Array.from(out).length, 30);
  assert.ok(out.startsWith("C:\\Users\\"));
  assert.ok(out.endsWith("thumb\\200x150x6x1"));
  assert.ok(out.includes("…"));
});

test("末尾の方に多く文字数を割り当てる", () => {
  const out = abbreviatePath("0123456789012345678901234567890123456789", 11);
  assert.equal(out, "0123…456789");
});

test("上限が1以下なら省略記号だけ", () => {
  assert.equal(abbreviatePath("abcdef", 1), "…");
  assert.equal(abbreviatePath("abcdef", 0), "…");
  assert.equal(abbreviatePath("abcdef", -3), "…");
});

test("上限が2なら末尾1文字だけ残す", () => {
  assert.equal(abbreviatePath("abcdef", 2), "…f");
});

test("サロゲートペアを途中で切らない", () => {
  const path = "𠮷".repeat(10);
  const out = abbreviatePath(path, 5);
  assert.equal(Array.from(out).length, 5);
  assert.ok(!out.includes("\uFFFD"));
  assert.doesNotMatch(out, /[\uD800-\uDBFF](?![\uDC00-\uDFFF])/);
});
