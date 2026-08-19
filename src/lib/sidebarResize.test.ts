// effectiveMaxWidth のユニットテスト。src/lib/paths.test.ts と同じパターン
// （Node組み込みtest runnerを`node --experimental-strip-types --test`で
// 実行、package.jsonのtest:unit参照）。
import assert from "node:assert/strict";
import { test } from "node:test";
import { effectiveMaxWidth, MIN_WIDTH, MAX_WIDTH } from "./sidebarResize.ts";

test("十分広いウィンドウ（1280px）ではMAX_WIDTH（500）が返る", () => {
  assert.equal(effectiveMaxWidth(1280), MAX_WIDTH);
  assert.equal(effectiveMaxWidth(1280), 500);
});

test("狭いウィンドウ（900px）では.video-listのmin-width(532px)と.appのpadding(32px)分だけ縮んだ値が返る", () => {
  // 900 - 32 (APP_HORIZONTAL_PADDING_PX) - 532 (VIDEO_LIST_MIN_WIDTH_PX) = 336
  assert.equal(effectiveMaxWidth(900), 336);
});

test("極端に狭いウィンドウ（400px）でもMIN_WIDTH（200）を下回らない", () => {
  // 400 - 32 - 532 = -164 (負数) だが、MIN_WIDTHでクランプされる
  assert.equal(effectiveMaxWidth(400), MIN_WIDTH);
  assert.equal(effectiveMaxWidth(400), 200);
});

test("MIN_WIDTHちょうどになる境界（764px = 32+532+200）", () => {
  assert.equal(effectiveMaxWidth(764), 200);
});

test("MAX_WIDTHちょうどになる境界（1064px = 32+532+500）", () => {
  assert.equal(effectiveMaxWidth(1064), 500);
  assert.equal(effectiveMaxWidth(1065), 500);
  assert.equal(effectiveMaxWidth(1063), 499);
});
