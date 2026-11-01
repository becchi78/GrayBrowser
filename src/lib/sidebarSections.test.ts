// toggleSection のユニットテスト。src/lib/ratingBar.test.ts と
// 同じパターン(package.jsonのtest:unit参照)。
import assert from "node:assert/strict";
import { test } from "node:test";
import {
  DEFAULT_SIDEBAR_SECTIONS_COLLAPSED,
  toggleSection,
} from "./sidebarSections.ts";

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
