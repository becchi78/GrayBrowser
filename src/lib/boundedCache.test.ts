// setWithEviction のユニットテスト。
// プロジェクトにvitest等のフロントエンド向けテストランナーが未導入のため、
// e2eテスト（e2e/specs/*.e2e.mjs）と同様にNode組み込みのtest runnerを使う。
// 型注釈を含む.tsファイルをそのまま実行できるよう、
// `node --experimental-strip-types --test` で実行する想定（package.jsonのtest:unit参照）。
import assert from "node:assert/strict";
import { test } from "node:test";
import { setWithEviction } from "./boundedCache.ts";

test("上限未満なら何も削除されない", () => {
  const cache = new Map<string, string>();
  setWithEviction(cache, "a", "1", 3);
  setWithEviction(cache, "b", "2", 3);

  assert.equal(cache.size, 2);
  assert.equal(cache.get("a"), "1");
  assert.equal(cache.get("b"), "2");
});

test("上限を超えたら最も古いエントリから削除され、サイズが上限を超えない", () => {
  const cache = new Map<string, string>();
  setWithEviction(cache, "a", "1", 2);
  setWithEviction(cache, "b", "2", 2);
  setWithEviction(cache, "c", "3", 2);

  assert.equal(cache.size, 2);
  assert.equal(cache.has("a"), false, "最も古い a は削除されているはず");
  assert.equal(cache.get("b"), "2");
  assert.equal(cache.get("c"), "3");
});

test("上限を1件ずつ超える追加を繰り返してもサイズは常に上限以下", () => {
  const cache = new Map<string, string>();
  const maxEntries = 5;
  for (let i = 0; i < 100; i++) {
    setWithEviction(cache, `key-${i}`, `value-${i}`, maxEntries);
    assert.ok(cache.size <= maxEntries);
  }
  // 直近maxEntries件だけが残っているはず
  for (let i = 95; i < 100; i++) {
    assert.equal(cache.get(`key-${i}`), `value-${i}`);
  }
  for (let i = 0; i < 95; i++) {
    assert.equal(cache.has(`key-${i}`), false);
  }
});

test("既存キーへの再setはエントリ数を増やさない（Mapの挙動）", () => {
  const cache = new Map<string, string>();
  setWithEviction(cache, "a", "1", 2);
  setWithEviction(cache, "b", "2", 2);
  // 既存キー a を更新: サイズは2のまま
  setWithEviction(cache, "a", "1-updated", 2);

  assert.equal(cache.size, 2);
  assert.equal(cache.get("a"), "1-updated");
  assert.equal(cache.get("b"), "2");
});
