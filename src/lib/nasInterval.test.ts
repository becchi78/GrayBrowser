// parseIntervalMinutes のユニットテスト。src/lib/ratingBar.test.ts と同じパターン
// (package.json の test:unit 参照)。
import assert from "node:assert/strict";
import { test } from "node:test";
import { DEFAULT_MINUTES, MAX_MINUTES, MIN_MINUTES, parseIntervalMinutes } from "./nasInterval.ts";

test("定数はバックエンドと同じ値", () => {
  assert.equal(MIN_MINUTES, 1);
  assert.equal(MAX_MINUTES, 1440);
  assert.equal(DEFAULT_MINUTES, 10);
});

test("境界値 1 と 1440 は受理される", () => {
  assert.deepEqual(parseIntervalMinutes("1"), { ok: true, minutes: 1 });
  assert.deepEqual(parseIntervalMinutes("1440"), { ok: true, minutes: 1440 });
});

test("0 と 1441 は拒否される", () => {
  assert.equal(parseIntervalMinutes("0").ok, false);
  assert.equal(parseIntervalMinutes("1441").ok, false);
});

test("負数は拒否される", () => {
  assert.equal(parseIntervalMinutes("-1").ok, false);
});

test("空文字と空白のみは拒否される", () => {
  assert.equal(parseIntervalMinutes("").ok, false);
  assert.equal(parseIntervalMinutes("   ").ok, false);
});

test("小数・指数表記・数字以外・全角は拒否される", () => {
  for (const s of ["1.5", "1e3", "abc", "１０", "0x10", "+5", "1 0"]) {
    assert.equal(parseIntervalMinutes(s).ok, false, s);
  }
});

test("前後の空白は trim して判定する", () => {
  assert.deepEqual(parseIntervalMinutes(" 10 "), { ok: true, minutes: 10 });
});

test("拒否時は理由の文言を返す", () => {
  const r = parseIntervalMinutes("0");
  assert.equal(r.ok, false);
  if (!r.ok) {
    assert.ok(r.reason.length > 0);
  }
});
