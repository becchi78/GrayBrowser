import assert from "node:assert/strict";
import { test } from "node:test";
import {
  overwriteNotice,
  unmatchedCountText,
  unmatchedKindLabel,
  unmatchedRowKey,
} from "./unmatchedThumbnails.ts";

test("原因の種類を表示名にする", () => {
  assert.equal(unmatchedKindLabel("empty_hash"), "空のhash");
  assert.equal(unmatchedKindLabel("no_matching_video"), "対応する動画なし");
});

test("件数は1件以上のときだけ文言にする", () => {
  assert.equal(unmatchedCountText(0), null);
  assert.equal(unmatchedCountText(-1), null);
  assert.equal(unmatchedCountText(99), "自動対応できなかった旧サムネイル: 99件");
});

test("行の識別子はフォルダとファイル名の組で決まる", () => {
  const a = unmatchedRowKey({ folder_path: "C:/a", file_name: "x.#.jpg" });
  assert.notEqual(a, unmatchedRowKey({ folder_path: "C:/b", file_name: "x.#.jpg" }));
  assert.notEqual(a, unmatchedRowKey({ folder_path: "C:/a", file_name: "y.#.jpg" }));
  assert.equal(a, unmatchedRowKey({ folder_path: "C:/a", file_name: "x.#.jpg" }));
  // 区切り文字を含む名前でも別の組と衝突しない。
  assert.notEqual(
    unmatchedRowKey({ folder_path: "a|b", file_name: "c" }),
    unmatchedRowKey({ folder_path: "a", file_name: "b|c" }),
  );
});

test("上書きの注意は既にサムネイルがある動画のときだけ出す", () => {
  assert.equal(overwriteNotice(false), null);
  assert.match(overwriteNotice(true) ?? "", /置き換え/);
});
