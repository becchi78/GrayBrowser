// effectiveMaxWidth のユニットテスト。
import assert from "node:assert/strict";
import { test } from "node:test";
import { clampPaneWidth, effectiveMaxWidth, MIN_WIDTH, MAX_WIDTH } from "./sidebarResize.ts";

test("十分広いウィンドウ（1280px）ではMAX_WIDTH（500）が返る", () => {
  assert.equal(effectiveMaxWidth(1280), MAX_WIDTH);
  assert.equal(effectiveMaxWidth(1280), 500);
});

test("狭いウィンドウ（900px）では.video-listのmin-width(598px)と.appのpadding(32px)分だけ縮んだ値が返る", () => {
  // 900 - 32 (APP_HORIZONTAL_PADDING_PX) - 598 (VIDEO_LIST_MIN_WIDTH_PX) = 270
  assert.equal(effectiveMaxWidth(900), 270);
});

test("極端に狭いウィンドウ（400px）でもMIN_WIDTH（200）を下回らない", () => {
  // 400 - 32 - 598 = -230 (負数) だが、MIN_WIDTHでクランプされる
  assert.equal(effectiveMaxWidth(400), MIN_WIDTH);
  assert.equal(effectiveMaxWidth(400), 200);
});

test("MIN_WIDTHちょうどになる境界（830px = 32+598+200）", () => {
  assert.equal(effectiveMaxWidth(830), 200);
});

test("MAX_WIDTHちょうどになる境界（1130px = 32+598+500）", () => {
  assert.equal(effectiveMaxWidth(1130), 500);
  assert.equal(effectiveMaxWidth(1131), 500);
  assert.equal(effectiveMaxWidth(1129), 499);
});

test("一覧の下限幅を渡すと、その値で上限が決まる(テーマごとに違う下限幅)", () => {
  // 900 - 32 - 556 = 312
  assert.equal(effectiveMaxWidth(900, 556), 312);
  // 下限幅が広いほど縮み、MIN_WIDTH で止まる
  assert.equal(effectiveMaxWidth(900, 700), MIN_WIDTH);
});

test("clampPaneWidth は MIN_WIDTH〜MAX_WIDTH に収める(テーマの初期幅 224 はそのまま)", () => {
  assert.equal(clampPaneWidth(224), 224);
  assert.equal(clampPaneWidth(MIN_WIDTH), MIN_WIDTH);
  assert.equal(clampPaneWidth(MAX_WIDTH), MAX_WIDTH);
  assert.equal(clampPaneWidth(MIN_WIDTH - 1), MIN_WIDTH);
  assert.equal(clampPaneWidth(MAX_WIDTH + 1), MAX_WIDTH);
});
