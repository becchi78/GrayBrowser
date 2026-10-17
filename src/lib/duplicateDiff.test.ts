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
    rating: 3,
    mtime: 1700000000,
    tags: ["a", "b"],
    ...overrides,
  };
}

const NONE = { file_size: false, duration: false, rating: false, tags: false, mtime: false };

test("全項目が一致すれば何も強調しない", () => {
  assert.deepEqual(computeDuplicateDiff([member(), member(), member()]), NONE);
});

test("値が異なる項目だけが true になる", () => {
  const diff = computeDuplicateDiff([member(), member({ file_size: 2000, rating: 0 })]);
  assert.deepEqual(diff, { ...NONE, file_size: true, rating: true });
});

test("3件のうち1件だけ異なっても不一致", () => {
  const diff = computeDuplicateDiff([member(), member(), member({ duration: 61 })]);
  assert.deepEqual(diff, { ...NONE, duration: true });
});

test("null を含む項目は全員 null でも不一致扱い", () => {
  const diff = computeDuplicateDiff([
    member({ duration: null, mtime: null }),
    member({ duration: null, mtime: null }),
  ]);
  assert.deepEqual(diff, { ...NONE, duration: true, mtime: true });
});

test("null と値が混在する場合も不一致", () => {
  const diff = computeDuplicateDiff([member({ mtime: null }), member()]);
  assert.deepEqual(diff, { ...NONE, mtime: true });
});

test("メンバー1件のグループは何も強調しない（null があっても）", () => {
  assert.deepEqual(computeDuplicateDiff([member({ duration: null, mtime: null })]), NONE);
});

test("メンバー0件でも何も強調しない", () => {
  assert.deepEqual(computeDuplicateDiff([]), NONE);
});

test("タグは順序が違っても同じ集合なら一致", () => {
  const diff = computeDuplicateDiff([member({ tags: ["a", "b"] }), member({ tags: ["b", "a"] })]);
  assert.equal(diff.tags, false);
});

test("タグの内容が違えば不一致、空配列と非空も不一致", () => {
  assert.equal(computeDuplicateDiff([member({ tags: ["a"] }), member({ tags: ["b"] })]).tags, true);
  assert.equal(computeDuplicateDiff([member({ tags: [] }), member({ tags: ["a"] })]).tags, true);
  assert.equal(computeDuplicateDiff([member({ tags: [] }), member({ tags: [] })]).tags, false);
});
