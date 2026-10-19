// toggleSection / deriveTagLists のユニットテスト。src/lib/ratingBar.test.ts と
// 同じパターン(package.jsonのtest:unit参照)。
import assert from "node:assert/strict";
import { test } from "node:test";
import {
  DEFAULT_SIDEBAR_SECTIONS_COLLAPSED,
  deriveTagLists,
  toggleSection,
} from "./sidebarSections.ts";

const tag = (id: number) => ({ id, name: `t${id}` });
const ids = (tags: { id: number }[]) => tags.map((t) => t.id);

test("既定はすべて展開", () => {
  assert.deepEqual(DEFAULT_SIDEBAR_SECTIONS_COLLAPSED, {
    folders: false,
    rating: false,
    tags: false,
  });
});

test("toggleSectionは指定セクションだけ反転し、元の状態は変更しない", () => {
  const before = { folders: false, rating: false, tags: false };
  const after = toggleSection(before, "rating");
  assert.deepEqual(after, { folders: false, rating: true, tags: false });
  assert.deepEqual(before, { folders: false, rating: false, tags: false });
  assert.deepEqual(toggleSection(after, "rating"), before);
});

test("常設がなければallTagsの順のまま", () => {
  assert.deepEqual(
    ids(deriveTagLists([tag(1), tag(2), tag(3)], [])),
    [1, 2, 3],
  );
});

test("常設タグは常設の順に先頭へ並び、残りはallTagsの順で続く", () => {
  const all = [tag(1), tag(2), tag(3), tag(4)];
  assert.deepEqual(ids(deriveTagLists(all, [3, 1])), [3, 1, 2, 4]);
});

test("存在しないタグIDと常設内の重複は無視される", () => {
  const all = [tag(1), tag(2)];
  assert.deepEqual(ids(deriveTagLists(all, [99, 2, 2])), [2, 1]);
});

test("タグが0件なら空", () => {
  assert.deepEqual(deriveTagLists([], [1]), []);
});
