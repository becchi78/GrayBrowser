// needsPropertiesRefresh のユニットテスト。src/lib/ratingBar.test.ts と
// 同じパターン(package.jsonのtest:unit参照)。
import assert from "node:assert/strict";
import { test } from "node:test";
import { needsPropertiesRefresh } from "./videoProperties.ts";
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
