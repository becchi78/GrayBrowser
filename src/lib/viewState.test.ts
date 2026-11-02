// viewState のユニットテスト。package.json の test:unit 参照。
import assert from "node:assert/strict";
import { test } from "node:test";
import {
  INITIAL_VIEW_STATE,
  fromSaved,
  isSameViewState,
  toSaved,
  type ViewState,
} from "./viewState.ts";

const sample: ViewState = {
  search: "abc",
  folderPath: "/a",
  sortField: "rating",
  sortDirection: "desc",
  tagIds: [2, 1],
  ratings: [3, 5],
};

test("保存形式との往復で同じ状態に戻る", () => {
  assert.deepEqual(fromSaved(toSaved(sample)), sample);
  assert.deepEqual(fromSaved(toSaved(INITIAL_VIEW_STATE)), INITIAL_VIEW_STATE);
});

test("保存形式の項目名はバックエンドと一致する", () => {
  assert.deepEqual(toSaved(sample), {
    search: "abc",
    folder_path: "/a",
    sort_field: "rating",
    sort_direction: "desc",
    tag_ids: [2, 1],
    ratings: [3, 5],
  });
});

test("同じ内容なら同一と判定する", () => {
  assert.equal(isSameViewState(sample, { ...sample, tagIds: [2, 1], ratings: [3, 5] }), true);
});

test("各項目の違いを検知する", () => {
  assert.equal(isSameViewState(sample, { ...sample, search: "x" }), false);
  assert.equal(isSameViewState(sample, { ...sample, folderPath: null }), false);
  assert.equal(isSameViewState(sample, { ...sample, sortField: "folder" }), false);
  assert.equal(isSameViewState(sample, { ...sample, sortDirection: "asc" }), false);
  assert.equal(isSameViewState(sample, { ...sample, tagIds: [1, 2] }), false);
  assert.equal(isSameViewState(sample, { ...sample, tagIds: [2] }), false);
  assert.equal(isSameViewState(sample, { ...sample, ratings: [] }), false);
});
