// createLatestOnlyGuard のユニットテスト。src/lib/ratingBar.test.ts と
// 同じパターン(Node組み込みtest runnerを
// `node --experimental-strip-types --test`で実行、package.jsonのtest:unit
// 参照)。
import assert from "node:assert/strict";
import { test } from "node:test";
import { createLatestOnlyGuard } from "./latestRequest.ts";

test("最初に発行した連番は、他に発行がなければ最新", () => {
  const guard = createLatestOnlyGuard();
  assert.equal(guard.isLatest(guard.begin()), true);
});

test("新しい連番を発行すると、古い連番は最新ではなくなる", () => {
  const guard = createLatestOnlyGuard();
  const first = guard.begin();
  const second = guard.begin();
  assert.equal(guard.isLatest(first), false);
  assert.equal(guard.isLatest(second), true);
});

test("連続して発行しても、最後の連番だけが最新", () => {
  const guard = createLatestOnlyGuard();
  const ids = [guard.begin(), guard.begin(), guard.begin(), guard.begin()];
  assert.deepEqual(
    ids.map((id) => guard.isLatest(id)),
    [false, false, false, true],
  );
});

test("発行前の連番は最新ではない", () => {
  const guard = createLatestOnlyGuard();
  assert.equal(guard.isLatest(0), false);
  assert.equal(guard.isLatest(1), false);
});

test("ガードごとに連番は独立している", () => {
  const a = createLatestOnlyGuard();
  const b = createLatestOnlyGuard();
  const aId = a.begin();
  b.begin();
  b.begin();
  assert.equal(a.isLatest(aId), true);
});
