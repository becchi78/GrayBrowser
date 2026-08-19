// splitDirAndFileName のユニットテスト。
// src/lib/boundedCache.test.ts と同じパターン（Node組み込みtest runnerを
// `node --experimental-strip-types --test` で実行、package.jsonのtest:unit参照）。
import assert from "node:assert/strict";
import { test } from "node:test";
import { splitDirAndFileName } from "./paths.ts";

test("通常のWindowsパス（\\区切り）は最後の区切り文字でdirとnameに分割される", () => {
  const { dir, name } = splitDirAndFileName("C:\\Users\\testuser\\Videos\\sample.mp4");
  assert.equal(dir, "C:\\Users\\testuser\\Videos\\");
  assert.equal(name, "sample.mp4");
});

test("file_nameという第二引数を持たない設計のため、大文字小文字がfile_nameと食い違うようなパスでも正しく分割できる", () => {
  // かつての endsWith(fileName) 方式では、file_path の末尾が実際の
  // file_name と大小文字で食い違う実データ（Windowsはファイル名の大小文字を
  // 区別しない）で name が空文字列になるフォールバックを踏んでいた。
  // filePath単体を区切り文字で分割するこの方式では、file_nameとの突き合わせが
  // 不要なため、この種の不一致は原理的に発生しない。
  const { dir, name } = splitDirAndFileName("C:\\Video\\A.MP4");
  assert.equal(dir, "C:\\Video\\");
  assert.equal(name, "A.MP4");
});

test("/を含むパスも最後の区切り文字で分割される", () => {
  const { dir, name } = splitDirAndFileName("/mnt/videos/deep/nested/sample.mp4");
  assert.equal(dir, "/mnt/videos/deep/nested/");
  assert.equal(name, "sample.mp4");
});

test("区切り文字を含まない文字列は、dirが空でnameに全体が入る", () => {
  const { dir, name } = splitDirAndFileName("sample.mp4");
  assert.equal(dir, "");
  assert.equal(name, "sample.mp4");
});

test("末尾が区切り文字の異常入力は、dirが空でnameに全体が入る（安全側に倒す）", () => {
  const { dir, name } = splitDirAndFileName("C:\\Users\\testuser\\Videos\\");
  assert.equal(dir, "");
  assert.equal(name, "C:\\Users\\testuser\\Videos\\");
});
