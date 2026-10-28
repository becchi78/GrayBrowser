// buildWbImportResultDisplay のユニットテスト。
// src/lib/boundedCache.test.ts と同じNode組み込みtest runnerパターン。
import assert from "node:assert/strict";
import { test } from "node:test";
import { buildWbImportResultDisplay } from "./wbImportResultDisplay.ts";
import type { WbImportSummary } from "../types.ts";

function makeSummary(overrides: Partial<WbImportSummary>): WbImportSummary {
  return {
    registered: 0,
    skipped: 0,
    skipped_invalid_name: 0,
    clamped_scores: 0,
    tags_assigned: 0,
    source_tag_count: 0,
    ratings_from_star_tags: 0,
    thumbnails_linked: 0,
    thumbnails_failed: 0,
    thumbnails_unmatched: 0,
    thumbnails_skipped_online: 0,
    watch_folders_added: [],
    ...overrides,
  };
}

const keys = (items: { key: string }[]) => items.map((i) => i.key);

test("要対応なし・内訳なしなら、登録件数だけで読み切れる", () => {
  const d = buildWbImportResultDisplay(makeSummary({ registered: 126 }));
  assert.equal(d.registeredText, "登録: 126件");
  assert.equal(d.watchFoldersAddedText, null);
  assert.deepEqual(d.attention, []);
  assert.deepEqual(d.details, []);
  assert.equal(d.showOnlineThumbnailNote, false);
});

test("監視フォルダを追加したときだけ件数行が出る", () => {
  const d = buildWbImportResultDisplay(
    makeSummary({ registered: 1, watch_folders_added: ["D:\\a", "D:\\b"] }),
  );
  assert.equal(d.watchFoldersAddedText, "監視フォルダに追加: 2件");
  assert.deepEqual(d.watchFolders, ["D:\\a", "D:\\b"]);
});

test("オンライン動画の旧サムネイルは要対応ではなく内訳に入る", () => {
  const d = buildWbImportResultDisplay(
    makeSummary({ registered: 126, thumbnails_skipped_online: 82 }),
  );
  assert.deepEqual(d.attention, []);
  assert.deepEqual(keys(d.details), ["skipped-online"]);
  assert.match(d.details[0].text, /82件/);
  assert.equal(d.showOnlineThumbnailNote, true);
});

test("未対応・変換失敗・機種依存文字は要対応に出て、確認ボタンは未対応だけ", () => {
  const d = buildWbImportResultDisplay(
    makeSummary({
      registered: 5,
      thumbnails_unmatched: 3,
      thumbnails_failed: 2,
      skipped_invalid_name: 4,
    }),
  );
  assert.deepEqual(keys(d.attention), ["unmatched", "failed", "invalid-name"]);
  assert.deepEqual(
    d.attention.map((a) => a.expandable),
    [true, false, false],
  );
  assert.equal(d.attention[0].tone, "amber");
  assert.equal(d.attention[1].tone, "failed");
  assert.match(d.attention[1].text, /2件/);
  assert.match(d.attention[2].text, /4件/);
  assert.deepEqual(d.details, []);
});

test("タグ付与は成功なら内訳、amber/failed なら要対応", () => {
  const ok = buildWbImportResultDisplay(
    makeSummary({ registered: 3, tags_assigned: 6, source_tag_count: 6 }),
  );
  assert.deepEqual(keys(ok.details), ["tags"]);
  assert.deepEqual(ok.attention, []);

  const amber = buildWbImportResultDisplay(
    makeSummary({ registered: 3, skipped: 1, tags_assigned: 0, source_tag_count: 6 }),
  );
  assert.deepEqual(keys(amber.attention), ["tags"]);
  assert.equal(amber.attention[0].tone, "amber");

  const failed = buildWbImportResultDisplay(makeSummary({ source_tag_count: 6 }));
  assert.deepEqual(keys(failed.attention), ["tags"]);
  assert.equal(failed.attention[0].tone, "failed");
});

test("タグ0件の説明つき表示（muted）は件数0なので出さない", () => {
  const noTags = buildWbImportResultDisplay(makeSummary({ registered: 3 }));
  assert.deepEqual(noTags.attention, []);
  assert.deepEqual(noTags.details, []);

  const allSkipped = buildWbImportResultDisplay(
    makeSummary({ skipped: 3, source_tag_count: 5 }),
  );
  assert.deepEqual(allSkipped.attention, []);
  assert.deepEqual(keys(allSkipped.details), ["skipped"]);
});

test("全行が機種依存文字で未登録でも、タグ付与の失敗とは表示しない", () => {
  const d = buildWbImportResultDisplay(
    makeSummary({ skipped_invalid_name: 3, source_tag_count: 5 }),
  );
  assert.deepEqual(keys(d.attention), ["invalid-name"]);
});

test("内訳は0件の項目を出さず、すべて非0なら決まった順で並ぶ", () => {
  const d = buildWbImportResultDisplay(
    makeSummary({
      registered: 10,
      skipped: 2,
      clamped_scores: 1,
      tags_assigned: 7,
      source_tag_count: 7,
      ratings_from_star_tags: 4,
      thumbnails_linked: 41,
      thumbnails_skipped_online: 5,
    }),
  );
  assert.deepEqual(keys(d.details), [
    "tags",
    "star-ratings",
    "skipped-online",
    "linked",
    "skipped",
    "clamped",
  ]);

  const partial = buildWbImportResultDisplay(makeSummary({ registered: 1, clamped_scores: 2 }));
  assert.deepEqual(keys(partial.details), ["clamped"]);
});
