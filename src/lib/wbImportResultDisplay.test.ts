// buildWbImportResultDisplay のユニットテスト。
// src/lib/boundedCache.test.ts と同じNode組み込みtest runnerパターン。
import assert from "node:assert/strict";
import { test } from "node:test";
import { buildWbImportResultDisplay } from "./wbImportResultDisplay.ts";
import type { WbImportSummary } from "../types.ts";

function makeSummary(overrides: Partial<WbImportSummary>): WbImportSummary {
  return {
    total: 0,
    registered: 0,
    registered_online: 0,
    registered_offline: 0,
    failed_rows: 0,
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

test("主表示は .wb内の動画 とインポート(オンライン/オフライン)の内訳", () => {
  const d = buildWbImportResultDisplay(
    makeSummary({ total: 126, registered: 126, registered_online: 85, registered_offline: 41 }),
  );
  assert.equal(d.totalText, ".wb内の動画: 126件");
  assert.equal(d.registeredText, "インポート: 126件（オンライン 85件 / オフライン 41件）");
  assert.equal(d.watchFoldersAddedText, null);
  assert.equal(d.skippedText, null);
  assert.deepEqual(d.attention, []);
  assert.deepEqual(d.details, []);
});

test("オフラインが1件以上のとき、旧サムネイル紐付け成功をオフライン件数に対して示す", () => {
  const d = buildWbImportResultDisplay(
    makeSummary({
      total: 10,
      registered: 10,
      registered_online: 4,
      registered_offline: 6,
      thumbnails_linked: 5,
    }),
  );
  assert.equal(d.offlineThumbnailsText, "オフライン動画の旧サムネイル紐付け成功: 5件（6件中）");
});

test("オフラインが0件なら旧サムネイル紐付けの行は出さない", () => {
  const d = buildWbImportResultDisplay(
    makeSummary({ total: 3, registered: 3, registered_online: 3 }),
  );
  assert.equal(d.offlineThumbnailsText, null);
});

test("オフラインがあって紐付けが0件なら 0件（Y件中）と出す", () => {
  const d = buildWbImportResultDisplay(
    makeSummary({ total: 2, registered: 2, registered_offline: 2, thumbnails_linked: 0 }),
  );
  assert.equal(d.offlineThumbnailsText, "オフライン動画の旧サムネイル紐付け成功: 0件（2件中）");
});

test("監視フォルダを追加したときだけ件数行が出る", () => {
  const d = buildWbImportResultDisplay(
    makeSummary({ registered: 1, watch_folders_added: ["D:\\a", "D:\\b"] }),
  );
  assert.equal(d.watchFoldersAddedText, "監視フォルダに追加: 2件");
  assert.deepEqual(d.watchFolders, ["D:\\a", "D:\\b"]);
});

test("オンライン動画の旧サムネイルは、件数も注記も一切出さない", () => {
  const d = buildWbImportResultDisplay(
    makeSummary({
      total: 126,
      registered: 126,
      registered_online: 126,
      thumbnails_skipped_online: 82,
    }),
  );
  assert.deepEqual(d.attention, []);
  assert.deepEqual(d.details, []);
  assert.ok(!("showOnlineThumbnailNote" in d));
  const all = JSON.stringify(d);
  assert.doesNotMatch(all, /移行対象外/);
  assert.doesNotMatch(all, /オンライン動画の旧サムネイル/);
  assert.doesNotMatch(all, /バックグラウンド/);
});

test("登録済みのためスキップした行は情報として出し、0件なら出さない", () => {
  const d = buildWbImportResultDisplay(makeSummary({ total: 8, skipped: 8 }));
  assert.equal(d.skippedText, "登録済みのためスキップ: 8件");
  assert.deepEqual(d.attention, []);
});

test("全行が登録済みの再インポートは、インポート 0件 とスキップ件数で等式が読める", () => {
  const d = buildWbImportResultDisplay(
    makeSummary({ total: 126, skipped: 126, source_tag_count: 40 }),
  );
  assert.equal(d.totalText, ".wb内の動画: 126件");
  assert.equal(d.registeredText, "インポート: 0件（オンライン 0件 / オフライン 0件）");
  assert.equal(d.offlineThumbnailsText, null);
  assert.equal(d.skippedText, "登録済みのためスキップ: 126件");
  // タグが0件なのは再インポートだからで、要対応にはならない。
  assert.deepEqual(d.attention, []);
  assert.deepEqual(d.details, []);
});

test("未対応・変換失敗・機種依存文字・取り込めなかった行は要対応に出て、確認ボタンは未対応だけ", () => {
  const d = buildWbImportResultDisplay(
    makeSummary({
      total: 14,
      registered: 5,
      registered_online: 5,
      thumbnails_unmatched: 3,
      thumbnails_failed: 2,
      skipped_invalid_name: 4,
      failed_rows: 5,
    }),
  );
  assert.deepEqual(keys(d.attention), ["unmatched", "failed", "invalid-name", "failed-rows"]);
  assert.deepEqual(
    d.attention.map((a) => a.expandable),
    [true, false, false, false],
  );
  assert.deepEqual(
    d.attention.map((a) => a.tone),
    ["amber", "failed", "amber", "failed"],
  );
  assert.match(d.attention[1].text, /2件/);
  assert.match(d.attention[2].text, /^機種依存文字のため未登録: 4件/);
  assert.equal(d.attention[3].text, "取り込めなかった行: 5件");
  assert.deepEqual(d.details, []);
});

test("タグは成功なら詳細（タグをインポート）、amber/failed なら要対応", () => {
  const ok = buildWbImportResultDisplay(
    makeSummary({ total: 3, registered: 3, tags_assigned: 6, source_tag_count: 6 }),
  );
  assert.deepEqual(keys(ok.details), ["tags"]);
  assert.equal(ok.details[0].text, "タグをインポート: 6件");
  assert.deepEqual(ok.attention, []);

  const amber = buildWbImportResultDisplay(
    makeSummary({ total: 4, registered: 3, skipped: 1, tags_assigned: 0, source_tag_count: 6 }),
  );
  assert.deepEqual(keys(amber.attention), ["tags"]);
  assert.equal(amber.attention[0].tone, "amber");

  const failed = buildWbImportResultDisplay(
    makeSummary({ total: 2, failed_rows: 2, source_tag_count: 6 }),
  );
  assert.deepEqual(keys(failed.attention), ["failed-rows", "tags"]);
  assert.equal(failed.attention[1].tone, "failed");
});

test("タグ0件の説明つき表示（muted）は件数0なので出さない", () => {
  const noTags = buildWbImportResultDisplay(makeSummary({ total: 3, registered: 3 }));
  assert.deepEqual(noTags.attention, []);
  assert.deepEqual(noTags.details, []);
});

test("全行が機種依存文字で未登録でも、タグの失敗とは表示しない", () => {
  const d = buildWbImportResultDisplay(
    makeSummary({ total: 3, skipped_invalid_name: 3, source_tag_count: 5 }),
  );
  assert.deepEqual(keys(d.attention), ["invalid-name"]);
});

test("★タグから評価にした件数は表示しない", () => {
  const d = buildWbImportResultDisplay(
    makeSummary({ total: 5, registered: 5, ratings_from_star_tags: 4 }),
  );
  assert.deepEqual(d.details, []);
  assert.doesNotMatch(JSON.stringify(d), /★/);
});

test("詳細は0件の項目を出さず、非0なら決まった順で並ぶ", () => {
  const d = buildWbImportResultDisplay(
    makeSummary({
      total: 10,
      registered: 10,
      clamped_scores: 1,
      tags_assigned: 7,
      source_tag_count: 7,
    }),
  );
  assert.deepEqual(keys(d.details), ["tags", "clamped"]);

  const partial = buildWbImportResultDisplay(makeSummary({ registered: 1, clamped_scores: 2 }));
  assert.deepEqual(keys(partial.details), ["clamped"]);
});
