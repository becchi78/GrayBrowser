// computeDuplicateDiff のユニットテスト。
// src/lib/paths.test.ts と同じパターン（package.jsonのtest:unit参照）。
import assert from "node:assert/strict";
import { test } from "node:test";
import { computeDuplicateDiff } from "./duplicateDiff.ts";
import type { DuplicateGroupMember } from "../types.ts";

function member(overrides: Partial<DuplicateGroupMember> = {}): DuplicateGroupMember {
  return {
    video_id: "id",
    file_path: "/v/a.mp4",
    file_name: "a.mp4",
    file_size: 1000,
    status: "online",
    created_at: "2026-01-01 00:00:00",
    duration: 60,
    width: 1920,
    height: 1080,
    ...overrides,
  };
}

const NONE = { file_size: false, duration: false, resolution: false };

test("全項目が一致すれば何も強調しない", () => {
  assert.deepEqual(computeDuplicateDiff([member(), member(), member()]), NONE);
});

test("値が異なる項目だけが true になる", () => {
  const diff = computeDuplicateDiff([member(), member({ file_size: 2000, width: 1280 })]);
  assert.deepEqual(diff, { ...NONE, file_size: true, resolution: true });
});

test("3件のうち1件だけ異なっても不一致", () => {
  const diff = computeDuplicateDiff([member(), member(), member({ duration: 61 })]);
  assert.deepEqual(diff, { ...NONE, duration: true });
});

test("解像度は幅と高さのどちらか一方が違えば不一致", () => {
  assert.equal(computeDuplicateDiff([member(), member({ height: 720 })]).resolution, true);
  assert.equal(computeDuplicateDiff([member(), member({ width: 1280 })]).resolution, true);
});

test("null を含む項目は全員 null でも不一致扱い", () => {
  const diff = computeDuplicateDiff([
    member({ duration: null, width: null, height: null }),
    member({ duration: null, width: null, height: null }),
  ]);
  assert.deepEqual(diff, { ...NONE, duration: true, resolution: true });
});

test("幅または高さの片方だけ null でも解像度は不一致", () => {
  assert.equal(computeDuplicateDiff([member({ height: null }), member()]).resolution, true);
  assert.equal(computeDuplicateDiff([member({ width: null }), member()]).resolution, true);
});

test("null と値が混在する場合も不一致", () => {
  const diff = computeDuplicateDiff([member({ duration: null }), member()]);
  assert.deepEqual(diff, { ...NONE, duration: true });
});

test("メンバー1件のグループは何も強調しない（null があっても）", () => {
  assert.deepEqual(
    computeDuplicateDiff([member({ duration: null, width: null, height: null })]),
    NONE,
  );
});

test("メンバー0件でも何も強調しない", () => {
  assert.deepEqual(computeDuplicateDiff([]), NONE);
});
