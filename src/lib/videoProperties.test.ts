// needsPropertiesRefresh と thumbnailWidthPx のユニットテスト。src/lib/ratingBar.test.ts と
// 同じパターン(package.jsonのtest:unit参照)。
import assert from "node:assert/strict";
import { test } from "node:test";
import { needsPropertiesRefresh, thumbnailWidthPx } from "./videoProperties.ts";
import type { VideoPropertiesDto } from "../types";

function props(probedAt: string | null): VideoPropertiesDto {
  return {
    width: 1920,
    height: 1080,
    video_codec: "h264",
    audio_codec: "aac",
    bitrate: 5_000_000,
    fps: 30,
    probed_at: probedAt,
  };
}

test("未取得(null)は取り直し対象", () => {
  assert.equal(needsPropertiesRefresh(null), true);
});

test("probed_atがnullなら取り直し対象", () => {
  assert.equal(needsPropertiesRefresh(props(null)), true);
});

test("probed_atが入っていれば取り直し不要", () => {
  assert.equal(needsPropertiesRefresh(props("2026-01-01T00:00:00Z")), false);
});

function sized(width: number | null, height: number | null): VideoPropertiesDto {
  return { ...props("2026-01-01T00:00:00Z"), width, height };
}

test("thumbnailWidthPx: 未取得(null)は既定の200px", () => {
  assert.equal(thumbnailWidthPx(null), 200);
});

test("thumbnailWidthPx: 幅か高さが null なら既定の200px", () => {
  assert.equal(thumbnailWidthPx(sized(null, 1080)), 200);
  assert.equal(thumbnailWidthPx(sized(1920, null)), 200);
});

test("thumbnailWidthPx: 幅か高さが 0 なら既定の200px", () => {
  assert.equal(thumbnailWidthPx(sized(0, 1080)), 200);
  assert.equal(thumbnailWidthPx(sized(1920, 0)), 200);
});

test("thumbnailWidthPx: 高さ150pxに合わせて比率から幅を出す", () => {
  assert.equal(thumbnailWidthPx(sized(1920, 1080)), 267);
  assert.equal(thumbnailWidthPx(sized(1080, 1920)), 84);
  assert.equal(thumbnailWidthPx(sized(640, 480)), 200);
});

test("thumbnailWidthPx: 端数は四捨五入", () => {
  // 150 * 1 / 4 = 37.5 → 38、150 * 1 / 7 = 21.43 → 21
  assert.equal(thumbnailWidthPx(sized(1, 4)), 38);
  assert.equal(thumbnailWidthPx(sized(1, 7)), 21);
});
