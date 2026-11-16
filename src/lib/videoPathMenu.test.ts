import assert from "node:assert/strict";
import { test } from "node:test";
import {
  buildPathMenuItems,
  isUnresolvedPath,
  revealDisabledReason,
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

test("1件のオンライン動画は、開けない理由がない", () => {
  assert.equal(revealDisabledReason([target("a")]), null);
});

test("複数選択・オフライン・未解決・対象なしは、理由を返す", () => {
  assert.match(revealDisabledReason([target("a"), target("b")])!, /複数選択/);
  assert.match(revealDisabledReason([target("a", { offline: true })])!, /オフライン/);
  assert.match(revealDisabledReason([target("a", { file_path: "::NAS\\a.mp4" })])!, /解決/);
  assert.notEqual(revealDisabledReason([]), null);
});

test("未解決パスはオフラインでも未解決の理由を優先する", () => {
  const reason = revealDisabledReason([target("a", { file_path: "::NAS\\a.mp4", offline: true })]);
  assert.match(reason!, /解決/);
});

test("1件のとき、コピーと開くの2項目が有効で、押すと対応する操作を呼ぶ", () => {
  const r = recorder();
  const items = buildPathMenuItems([target("a")], r.actions);
  assert.deepEqual(
    items.map((i) => [i.label, i.disabled]),
    [
      ["パスをコピー", undefined],
      ["エクスプローラーで開く", false],
    ],
  );
  items[0].onSelect();
  items[1].onSelect();
  assert.deepEqual(r.copied, [["C:\\videos\\a.mp4"]]);
  assert.deepEqual(r.revealed, ["a"]);
});

test("複数選択では、コピーは全パスを渡し、開くは無効になる", () => {
  const r = recorder();
  const items = buildPathMenuItems([target("a"), target("b")], r.actions);
  assert.equal(items[0].label, "パスをコピー(2 件)");
  assert.equal(items[1].disabled, true);
  assert.match(items[1].label, /^エクスプローラーで開く\(複数選択/);
  items[0].onSelect();
  assert.deepEqual(r.copied, [["C:\\videos\\a.mp4", "C:\\videos\\b.mp4"]]);
});

test("オフラインでも、コピーは有効で開くだけが無効になる", () => {
  const items = buildPathMenuItems([target("a", { offline: true })], recorder().actions);
  assert.equal(items[0].disabled, undefined);
  assert.equal(items[1].disabled, true);
  assert.match(items[1].label, /オフラインのため開けません/);
});

test("未解決パスでも、コピーは有効で開くだけが無効になる", () => {
  const items = buildPathMenuItems(
    [target("a", { file_path: "::NAS\\a.mp4" })],
    recorder().actions,
  );
  assert.equal(items[0].disabled, undefined);
  assert.equal(items[1].disabled, true);
});

test("対象が空なら項目を出さない", () => {
  assert.deepEqual(buildPathMenuItems([], recorder().actions), []);
});

test("項目のラベルは重複しない(メニューは label を key にする)", () => {
  const items = buildPathMenuItems([target("a"), target("b")], recorder().actions);
  assert.equal(new Set(items.map((i) => i.label)).size, items.length);
});
