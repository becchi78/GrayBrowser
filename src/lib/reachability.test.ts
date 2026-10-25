// isUnderFolder / isPathUnreachable のユニットテスト。バックエンドの
// gb_core::paths のテストと同じ観点（大文字小文字・区切り境界・UNC/ドライブ文字）。
// src/lib/paths.test.ts と同じパターン（package.jsonのtest:unit参照）。
import assert from "node:assert/strict";
import { test } from "node:test";
import { isPathUnreachable, isUnderFolder } from "./reachability.ts";

test("フォルダ配下のパスは、大文字小文字を区別せず一致する", () => {
  assert.equal(isUnderFolder("W:\\video\\a.mp4", "W:\\video"), true);
  assert.equal(isUnderFolder("w:\\VIDEO\\Sub\\a.mp4", "W:\\video"), true);
  assert.equal(isUnderFolder("W:\\video\\a.mp4", "W:\\video\\"), true);
});

test("名前が前方一致するだけの別フォルダは配下ではない", () => {
  assert.equal(isUnderFolder("W:\\video2\\a.mp4", "W:\\video"), false);
  assert.equal(isUnderFolder("W:\\videos\\a.mp4", "W:\\video\\"), false);
  assert.equal(isUnderFolder("W:\\video", "W:\\video"), false);
});

test("ドライブ直下とUNCを扱える", () => {
  assert.equal(isUnderFolder("W:\\a.mp4", "W:\\"), true);
  assert.equal(isUnderFolder("W:\\x\\a.mp4", "W:"), true);
  assert.equal(isUnderFolder("V:\\a.mp4", "W:\\"), false);
  assert.equal(isUnderFolder("\\\\nas\\share\\v\\a.mp4", "\\\\NAS\\share"), true);
  assert.equal(isUnderFolder("\\\\nas\\share2\\a.mp4", "\\\\nas\\share"), false);
});

test("/ と \\ は同じ区切りとして扱う", () => {
  assert.equal(isUnderFolder("C:/videos/a.mp4", "C:\\videos"), true);
  assert.equal(isUnderFolder("C:\\videos\\a.mp4", "C:/videos/"), true);
  assert.equal(isUnderFolder("C:/videos2/a.mp4", "C:/videos"), false);
});

test("空のフォルダは何も含まない", () => {
  assert.equal(isUnderFolder("\\a.mp4", ""), false);
  assert.equal(isUnderFolder("C:\\a.mp4", "  "), false);
  assert.equal(isUnderFolder("C:\\a.mp4", "\\"), false);
});

test("isPathUnreachable はどれかの到達不能フォルダ配下なら真、なければ偽", () => {
  const folders = ["W:\\video", "V:\\"];
  assert.equal(isPathUnreachable("V:\\a.mp4", folders), true);
  assert.equal(isPathUnreachable("W:\\video\\sub\\a.mp4", folders), true);
  assert.equal(isPathUnreachable("W:\\video2\\a.mp4", folders), false);
  assert.equal(isPathUnreachable("C:\\a.mp4", folders), false);
  assert.equal(isPathUnreachable("W:\\video\\a.mp4", []), false);
});
