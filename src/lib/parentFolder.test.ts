import assert from "node:assert/strict";
import test from "node:test";
import { parentFolder } from "./parentFolder.ts";

test("通常のパスは親フォルダを返す", () => {
  assert.equal(parentFolder("C:\\videos\\a.mp4"), "C:\\videos");
});

test("サブフォルダ", () => {
  assert.equal(parentFolder("D:\\videos\\2024\\trip\\a.mp4"), "D:\\videos\\2024\\trip");
});

test("ドライブ直下は C:\\ を返す", () => {
  assert.equal(parentFolder("C:\\a.mp4"), "C:\\");
});

test("UNCパス", () => {
  assert.equal(parentFolder("\\\\server\\share\\dir\\a.mp4"), "\\\\server\\share\\dir");
  assert.equal(parentFolder("\\\\server\\share\\a.mp4"), "\\\\server\\share");
});

test("区切りなし・空は空文字", () => {
  assert.equal(parentFolder("a.mp4"), "");
  assert.equal(parentFolder(""), "");
});

test("末尾の区切りは無視する", () => {
  assert.equal(parentFolder("C:\\videos\\sub\\"), "C:\\videos");
});

test("スラッシュ区切りも扱う", () => {
  assert.equal(parentFolder("C:/videos/a.mp4"), "C:/videos");
  assert.equal(parentFolder("C:/a.mp4"), "C:/");
});

test("機種依存文字を含むパス", () => {
  assert.equal(parentFolder("E:\\動画\\①まとめ\\ｱ㈱.mp4"), "E:\\動画\\①まとめ");
  assert.equal(parentFolder("E:\\動画\\𠮷野家.mp4"), "E:\\動画");
});
