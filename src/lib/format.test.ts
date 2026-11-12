// 表示用の整形関数のユニットテスト。src/lib/ratingBar.test.ts と
// 同じパターン(package.jsonのtest:unit参照)。
import assert from "node:assert/strict";
import { test } from "node:test";
import {
  formatBitrate,
  formatDuration,
  formatFileSize,
  formatFps,
  formatResolution,
} from "./format.ts";

test("formatBitrate: null は不明", () => {
  assert.equal(formatBitrate(null), "不明");
});

test("formatBitrate: Mbps に換算して小数1桁", () => {
  assert.equal(formatBitrate(4_500_000), "4.5 Mbps");
  assert.equal(formatBitrate(1_000_000), "1.0 Mbps");
  assert.equal(formatBitrate(1_234_567), "1.2 Mbps");
  assert.equal(formatBitrate(1_250_001), "1.3 Mbps");
});

test("formatBitrate: 0 は不明ではなく 0.0 Mbps", () => {
  assert.equal(formatBitrate(0), "0.0 Mbps");
});

test("formatFps: null は不明", () => {
  assert.equal(formatFps(null), "不明");
});

test("formatFps: 小数2桁", () => {
  assert.equal(formatFps(29.97), "29.97 fps");
  assert.equal(formatFps(30), "30.00 fps");
  assert.equal(formatFps(23.976), "23.98 fps");
});

test("formatFps: 0 は不明ではなく 0.00 fps", () => {
  assert.equal(formatFps(0), "0.00 fps");
});

test("formatDuration: null は不明、1時間未満は m:ss、以上は h:mm:ss", () => {
  assert.equal(formatDuration(null), "不明");
  assert.equal(formatDuration(0), "0:00");
  assert.equal(formatDuration(125), "2:05");
  assert.equal(formatDuration(3725.4), "1:02:05");
});

test("formatFileSize: 1GB 未満は MB、以上は GB", () => {
  assert.equal(formatFileSize(5 * 1024 * 1024), "5.0 MB");
  assert.equal(formatFileSize(1024 * 1024 * 1024), "1.00 GB");
});

test("formatResolution: どちらかが null なら不明", () => {
  assert.equal(formatResolution(1920, 1080), "1920 × 1080");
  assert.equal(formatResolution(null, 1080), "不明");
  assert.equal(formatResolution(1920, null), "不明");
});
