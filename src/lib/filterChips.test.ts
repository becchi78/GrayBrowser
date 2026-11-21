// filterChips のユニットテスト。package.json の test:unit 参照。
import assert from "node:assert/strict";
import { test } from "node:test";
import { buildFilterChips, formatCount, hasActiveFilter } from "./filterChips.ts";

const names = new Map([
  [1, "猫"],
  [2, "犬"],
]);

test("何も絞り込んでいなければ非アクティブでチップも無い", () => {
  const sel = { folderPath: null, tagIds: [], ratings: [] };
  assert.equal(hasActiveFilter(sel), false);
  assert.deepEqual(buildFilterChips(sel, names), []);
});

test("フォルダ・タグ・★のどれか1つでもあればアクティブ", () => {
  assert.equal(hasActiveFilter({ folderPath: "C:\\a", tagIds: [], ratings: [] }), true);
  assert.equal(hasActiveFilter({ folderPath: null, tagIds: [1], ratings: [] }), true);
  assert.equal(hasActiveFilter({ folderPath: null, tagIds: [], ratings: [3] }), true);
});

test("フォルダ → タグ(選択順)→ ★(昇順)の順に並ぶ", () => {
  const chips = buildFilterChips(
    { folderPath: "C:\\a", tagIds: [2, 1], ratings: [5, 3] },
    names,
  );
  assert.deepEqual(chips, [
    { kind: "folder", label: "C:\\a" },
    { kind: "tag", id: 2, label: "犬" },
    { kind: "tag", id: 1, label: "猫" },
    { kind: "rating", value: 3, label: "★★★" },
    { kind: "rating", value: 5, label: "★★★★★" },
  ]);
});

test("名前の分からないタグはチップにしないが、アクティブではある", () => {
  const sel = { folderPath: null, tagIds: [99], ratings: [] };
  assert.deepEqual(buildFilterChips(sel, names), []);
  assert.equal(hasActiveFilter(sel), true);
});

test("入力の ratings を並べ替えて壊さない", () => {
  const ratings = [5, 3];
  buildFilterChips({ folderPath: null, tagIds: [], ratings }, names);
  assert.deepEqual(ratings, [5, 3]);
});

test("件数は3桁区切り", () => {
  assert.equal(formatCount(0), "0");
  assert.equal(formatCount(3580), "3,580");
});
