import assert from "node:assert/strict";
import { test } from "node:test";
import {
  countUnmatchedByKind,
  filterUnmatchedByKind,
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

test("kind を指定するとその原因の行だけに絞り、省略すると全件を返す", () => {
  const rows = [
    { kind: "empty_hash" as const, n: 1 },
    { kind: "no_matching_video" as const, n: 2 },
    { kind: "empty_hash" as const, n: 3 },
  ];
  assert.deepEqual(
    filterUnmatchedByKind(rows, "empty_hash").map((r) => r.n),
    [1, 3],
  );
  assert.deepEqual(
    filterUnmatchedByKind(rows, "no_matching_video").map((r) => r.n),
    [2],
  );
  assert.equal(filterUnmatchedByKind(rows), rows);
  assert.deepEqual(filterUnmatchedByKind([], "empty_hash"), []);
});

test("原因別の残数を数え、未知の kind は無視する", () => {
  assert.deepEqual(countUnmatchedByKind([]), { empty_hash: 0, no_matching_video: 0 });
  const rows = [
    { kind: "empty_hash" },
    { kind: "no_matching_video" },
    { kind: "empty_hash" },
    { kind: "something_new" },
  ] as unknown as { kind: "empty_hash" | "no_matching_video" }[];
  assert.deepEqual(countUnmatchedByKind(rows), { empty_hash: 2, no_matching_video: 1 });
});
