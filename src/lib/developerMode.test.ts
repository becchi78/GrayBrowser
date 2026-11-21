// isDevVersion のユニットテスト。
import assert from "node:assert/strict";
import { test } from "node:test";
import { isDevVersion } from "./developerMode.ts";

test("-dev.N で終わる版だけが開発版", () => {
  assert.equal(isDevVersion("0.18.0-dev.1"), true);
  assert.equal(isDevVersion("1.2.3-dev.42"), true);
});

test("安定版や他のプレリリース、形式の崩れた版は開発版ではない", () => {
  for (const v of ["0.18.0", "0.18.0-rc1", "0.18.0-dev", "0.18.0-dev.1.2", ""]) {
    assert.equal(isDevVersion(v), false, v);
  }
});
