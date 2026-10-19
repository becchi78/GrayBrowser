// logFilter のユニットテスト。src/lib/nasInterval.test.ts と同じパターン
// (package.json の test:unit 参照)。
import assert from "node:assert/strict";
import { test } from "node:test";
import { LOG_LEVELS, entryMatches, mergeEntries, singleLine } from "./logFilter.ts";

const ALL = new Set<string>(LOG_LEVELS);
const warn = { level: "WARN", target: "Graybrowser::Scan", message: "Found 3 Files" };

test("全レベルが選ばれていて検索語が空なら一致する", () => {
  assert.equal(entryMatches(warn, ALL, ""), true);
});

test("チェックされていないレベルは除外される", () => {
  assert.equal(entryMatches(warn, new Set(["ERROR"]), ""), false);
  assert.equal(entryMatches(warn, new Set(["WARN"]), ""), true);
  assert.equal(entryMatches(warn, new Set(), ""), false);
});

test("検索はメッセージとモジュール名に大文字小文字を区別せず部分一致する", () => {
  assert.equal(entryMatches(warn, ALL, "found 3"), true);
  assert.equal(entryMatches(warn, ALL, "SCAN"), true);
  assert.equal(entryMatches(warn, ALL, "thumbnail"), false);
});

test("検索語の前後の空白は無視され、空白だけなら全件一致", () => {
  assert.equal(entryMatches(warn, ALL, "  files "), true);
  assert.equal(entryMatches(warn, ALL, "   "), true);
});

test("日本語でも検索できる", () => {
  const e = { level: "ERROR", target: "m", message: "サムネイル生成に失敗" };
  assert.equal(entryMatches(e, ALL, "失敗"), true);
  assert.equal(entryMatches(e, ALL, "成功"), false);
});

test("レベル不明のエントリはレベルの絞り込みを通り、検索語は適用される", () => {
  const u = { level: "UNKNOWN", target: "", message: "fragment" };
  assert.equal(entryMatches(u, new Set(), ""), true);
  assert.equal(entryMatches(u, new Set(), "other"), false);
});

test("mergeEntries は末尾に足し、上限を超えた古い分を捨てる", () => {
  assert.deepEqual(mergeEntries([1, 2], [3], false, 5), [1, 2, 3]);
  assert.deepEqual(mergeEntries([1, 2, 3], [4, 5], false, 3), [3, 4, 5]);
  assert.deepEqual(mergeEntries([1, 2, 3], [], false, 3), [1, 2, 3]);
  assert.deepEqual(mergeEntries([1, 2], [3, 4, 5, 6], false, 3), [4, 5, 6]);
});

test("mergeEntries は reset で保持分を捨てて入れ替える", () => {
  assert.deepEqual(mergeEntries([1, 2, 3], [9], true, 5), [9]);
  assert.deepEqual(mergeEntries([1, 2, 3], [], true, 5), []);
});

test("singleLine は改行を記号に置き換える", () => {
  assert.equal(singleLine("a\nb\r\nc"), "a ⏎ b ⏎ c");
  assert.equal(singleLine("abc"), "abc");
});
