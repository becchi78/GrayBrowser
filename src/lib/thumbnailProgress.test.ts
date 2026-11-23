// formatThumbnailProgress のユニットテスト。src/lib/ratingBar.test.ts と同じ
// パターン(package.jsonのtest:unit参照)。
import assert from "node:assert/strict";
import { test } from "node:test";
import { formatThumbnailProgress, isFinalProgress } from "./thumbnailProgress.ts";

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

test("isFinalProgress: 完了とtotal==0は最終値、進行中は違う", () => {
  assert.equal(isFinalProgress({ done: 0, total: 0 }), true);
  assert.equal(isFinalProgress({ done: 5, total: 5 }), true);
  assert.equal(isFinalProgress({ done: 6, total: 5 }), true);
  assert.equal(isFinalProgress({ done: 4, total: 5 }), false);
  assert.equal(isFinalProgress({ done: 0, total: 5 }), false);
});
