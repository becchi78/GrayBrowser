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

test("選択も常設もなければprimaryは空、othersは全タグ", () => {
  const all = [tag(1), tag(2), tag(3)];
  const { primary, others } = deriveTagLists(all, [], []);
  assert.deepEqual(ids(primary), []);
  assert.deepEqual(ids(others), [1, 2, 3]);
});

test("常設タグは常設の順に並び、残りがothersになる", () => {
  const all = [tag(1), tag(2), tag(3), tag(4)];
  const { primary, others } = deriveTagLists(all, [3, 1], []);
  assert.deepEqual(ids(primary), [3, 1]);
  assert.deepEqual(ids(others), [2, 4]);
});

test("選択中の未常設タグは選択順で常設の前に昇格する", () => {
  const all = [tag(1), tag(2), tag(3), tag(4)];
  const { primary, others } = deriveTagLists(all, [1], [4, 2]);
  assert.deepEqual(ids(primary), [4, 2, 1]);
  assert.deepEqual(ids(others), [3]);
});

test("選択済みの常設タグは位置が変わらず重複もしない", () => {
  const all = [tag(1), tag(2), tag(3)];
  const { primary } = deriveTagLists(all, [1, 2, 3], [2]);
  assert.deepEqual(ids(primary), [1, 2, 3]);
});

test("存在しないタグIDは選択・常設どちらでも無視される", () => {
  const all = [tag(1), tag(2)];
  const { primary, others } = deriveTagLists(all, [99, 1], [98]);
  assert.deepEqual(ids(primary), [1]);
  assert.deepEqual(ids(others), [2]);
});

test("タグが0件なら両方空", () => {
  const { primary, others } = deriveTagLists([], [1], [2]);
  assert.deepEqual(primary, []);
  assert.deepEqual(others, []);
});
