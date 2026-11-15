// thumbnailPlaceholderState のユニットテスト。src/lib/thumbnailProgress.test.ts と同じ
// パターン(package.jsonのtest:unit参照)。
import assert from "node:assert/strict";
import { test } from "node:test";
import { thumbnailPlaceholderState } from "./thumbnailPlaceholder.ts";

test("失敗でもオフラインでもなければ生成中", () => {
  assert.equal(
    thumbnailPlaceholderState({ thumbnail_failed: false }, false),
    "generating",
  );
});

test("オフラインでサムネイル生成が失敗していなければオフライン", () => {
  assert.equal(
    thumbnailPlaceholderState({ thumbnail_failed: false }, true),
    "offline",
  );
});

test("生成失敗はオフラインでなければ失敗", () => {
  assert.equal(
    thumbnailPlaceholderState({ thumbnail_failed: true }, false),
    "failed",
  );
});

test("失敗かつオフラインのときは失敗が優先される", () => {
  assert.equal(
    thumbnailPlaceholderState({ thumbnail_failed: true }, true),
    "failed",
  );
});
