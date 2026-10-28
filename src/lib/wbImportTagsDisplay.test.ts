// classifyTagsAssigned のユニットテスト。
// src/lib/boundedCache.test.ts と同じNode組み込みtest runnerパターン
// （node --experimental-strip-types --test、package.jsonのtest:unit参照）。
import assert from "node:assert/strict";
import { test } from "node:test";
import { classifyTagsAssigned } from "./wbImportTagsDisplay.ts";
import type { WbImportSummary } from "../types.ts";

// 各テストで意味を持たせるのは registered/skipped/tags_assigned/source_tag_count のみ。
// 他フィールドは分類ロジックに関与しないため0で埋める。
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

test("tags_assigned > 0 なら success", () => {
  const s = makeSummary({ registered: 3, skipped: 0, tags_assigned: 3, source_tag_count: 5 });
  const result = classifyTagsAssigned(s);

  assert.equal(result.tone, "success");
  assert.match(result.text, /^タグをインポート: 3件/);
});

test("source_tag_count === 0 なら muted（元データにタグが無かった、ケース1）", () => {
  const s = makeSummary({ registered: 3, skipped: 0, tags_assigned: 0, source_tag_count: 0 });
  const result = classifyTagsAssigned(s);

  assert.equal(result.tone, "muted");
  assert.match(result.text, /元データにタグがありませんでした/);
});

// 2回目以降の.wbインポートで全行が既登録としてスキップされた場合、
// tags_assigned=0でも正常であり、amberの「要確認」警告を誤表示しては
// ならない、という回帰防止のためのテスト。
test("registered === 0 かつ skipped > 0 なら muted（全行既登録スキップ、ケース3・正常）", () => {
  const s = makeSummary({ registered: 0, skipped: 5, tags_assigned: 0, source_tag_count: 5 });
  const result = classifyTagsAssigned(s);

  assert.equal(result.tone, "muted");
  assert.match(result.text, /すべて既に登録済み/);
});

// これが今回のPlan差し戻しで追加された回帰対象: registered === 0 だけでは
// 「全行スキップ（正常）」と「全行失敗（異常）」を区別できない。
// skipped === 0 まで確認して初めて実失敗の推定として扱う。
test("registered === 0 かつ skipped === 0 かつ failed_rows > 0 なら failed（全行失敗、ケース4）", () => {
  const s = makeSummary({
    registered: 0,
    skipped: 0,
    failed_rows: 3,
    tags_assigned: 0,
    source_tag_count: 5,
  });
  const result = classifyTagsAssigned(s);

  assert.equal(result.tone, "failed");
  assert.match(result.text, /インポートに失敗した可能性があります/);
});

// 機種依存文字のファイル名は仕様どおり登録されないだけで、取り込みの失敗ではない。
test("全行が機種依存文字でスキップされた場合は failed にならず muted", () => {
  const s = makeSummary({
    registered: 0,
    skipped: 0,
    skipped_invalid_name: 5,
    tags_assigned: 0,
    source_tag_count: 5,
  });
  const result = classifyTagsAssigned(s);

  assert.equal(result.tone, "muted");
  assert.doesNotMatch(result.text, /インポートに失敗した可能性/);
  assert.match(result.text, /機種依存文字/);
});

test("機種依存文字スキップが0件で registered/skipped も0件、失敗行があるなら failed", () => {
  const s = makeSummary({
    registered: 0,
    skipped: 0,
    skipped_invalid_name: 0,
    failed_rows: 2,
    tags_assigned: 0,
    source_tag_count: 5,
  });

  assert.equal(classifyTagsAssigned(s).tone, "failed");
});

// 失敗行が無いのに何も入らなかった場合は「失敗した可能性」と言い切れないので amber。
test("取り込めなかった行が無い registered === 0 && skipped === 0 は amber", () => {
  const s = makeSummary({ registered: 0, skipped: 0, tags_assigned: 0, source_tag_count: 5 });

  assert.equal(classifyTagsAssigned(s).tone, "amber");
});

test("registered > 0 かつ tags_assigned === 0 の残余ケースは amber（要確認）", () => {
  const s = makeSummary({ registered: 2, skipped: 0, tags_assigned: 0, source_tag_count: 5 });
  const result = classifyTagsAssigned(s);

  assert.equal(result.tone, "amber");
  assert.match(result.text, /要確認/);
});
