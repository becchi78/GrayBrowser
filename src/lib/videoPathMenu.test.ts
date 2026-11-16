import assert from "node:assert/strict";
import { test } from "node:test";
import {
  buildPathMenuItems,
  canReveal,
  isUnresolvedPath,
  type PathMenuTarget,
} from "./videoPathMenu.ts";

const target = (id: string, over: Partial<PathMenuTarget> = {}): PathMenuTarget => ({
  id,
  file_path: `C:\\videos\\${id}.mp4`,
  offline: false,
  ...over,
});

function recorder() {
  const copied: string[][] = [];
  const revealed: string[] = [];
  return {
    copied,
    revealed,
    actions: {
      onCopy: (paths: string[]) => copied.push(paths),
      onReveal: (id: string) => revealed.push(id),
    },
  };
}

test("::で始まるパスだけを未解決とみなす", () => {
  assert.equal(isUnresolvedPath("::NAS\\a.mp4"), true);
  assert.equal(isUnresolvedPath("C:\\::a.mp4"), false);
  assert.equal(isUnresolvedPath("\\\\nas\\share\\a.mp4"), false);
  assert.equal(isUnresolvedPath(""), false);
});

test("1件のオンライン動画だけが開ける", () => {
  assert.equal(canReveal([target("a")]), true);
});

test("複数選択・オフライン・未解決・対象なしは開けない", () => {
  assert.equal(canReveal([target("a"), target("b")]), false);
  assert.equal(canReveal([target("a", { offline: true })]), false);
  assert.equal(canReveal([target("a", { file_path: "::NAS\\a.mp4" })]), false);
  assert.equal(canReveal([target("a", { file_path: "::NAS\\a.mp4", offline: true })]), false);
  assert.equal(canReveal([]), false);
});

test("1件のとき、コピーと開くの2項目が出て、押すと対応する操作を呼ぶ", () => {
  const r = recorder();
  const items = buildPathMenuItems([target("a")], r.actions);
  assert.deepEqual(
    items.map((i) => [i.label, i.disabled]),
    [
      ["パスをコピー", undefined],
      ["エクスプローラーで開く", undefined],
    ],
  );
  items[0].onSelect();
  items[1].onSelect();
  assert.deepEqual(r.copied, [["C:\\videos\\a.mp4"]]);
  assert.deepEqual(r.revealed, ["a"]);
});

test("複数選択では、コピーは全パスを渡し、開くは出さない", () => {
  const r = recorder();
  const items = buildPathMenuItems([target("a"), target("b")], r.actions);
  assert.deepEqual(
    items.map((i) => i.label),
    ["パスをコピー(2 件)"],
  );
  items[0].onSelect();
  assert.deepEqual(r.copied, [["C:\\videos\\a.mp4", "C:\\videos\\b.mp4"]]);
});

test("オフラインでも、コピーだけが出る", () => {
  const items = buildPathMenuItems([target("a", { offline: true })], recorder().actions);
  assert.deepEqual(
    items.map((i) => i.label),
    ["パスをコピー"],
  );
});

test("未解決パスでも、コピーだけが出る", () => {
  const items = buildPathMenuItems(
    [target("a", { file_path: "::NAS\\a.mp4" })],
    recorder().actions,
  );
  assert.deepEqual(
    items.map((i) => i.label),
    ["パスをコピー"],
  );
});

test("対象が空なら項目を出さない", () => {
  assert.deepEqual(buildPathMenuItems([], recorder().actions), []);
});

test("項目のラベルは重複しない(メニューは label を key にする)", () => {
  const items = buildPathMenuItems([target("a")], recorder().actions);
  assert.equal(new Set(items.map((i) => i.label)).size, items.length);
});
