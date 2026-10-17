// formatThumbnailProgress のユニットテスト。src/lib/ratingBar.test.ts と同じ
// パターン(package.jsonのtest:unit参照)。
import assert from "node:assert/strict";
import { test } from "node:test";
import { formatThumbnailProgress } from "./thumbnailProgress.ts";

test("totalが0なら表示しない(null)", () => {
  assert.equal(formatThumbnailProgress({ done: 0, total: 0 }), null);
});

test("進行中は件数つきの文字列を返す", () => {
  assert.equal(
    formatThumbnailProgress({ done: 89, total: 120 }),
    "サムネイル生成（89/120件）",
  );
});

test("開始直後(0件完了)でも表示する", () => {
  assert.equal(
    formatThumbnailProgress({ done: 0, total: 3 }),
    "サムネイル生成（0/3件）",
  );
});
