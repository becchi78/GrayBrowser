// parseRatingFilterValue のユニットテスト。src/lib/sidebarResize.test.ts と
// 同じパターン(Node組み込みtest runnerを
// `node --experimental-strip-types --test`で実行、package.jsonのtest:unit
// 参照)。
import assert from "node:assert/strict";
import { test } from "node:test";
import { parseRatingFilterValue } from "./ratingBar.ts";

test("空文字列(「すべて表示」)はnullを返す", () => {
  assert.equal(parseRatingFilterValue(""), null);
});

test("数値文字列はNumberに変換される", () => {
  assert.equal(parseRatingFilterValue("1"), 1);
  assert.equal(parseRatingFilterValue("3"), 3);
  assert.equal(parseRatingFilterValue("5"), 5);
});
