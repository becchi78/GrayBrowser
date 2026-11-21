// TagBarEditDialog の並び替え(↑/↓ごとに即保存)と閉じるボタンを確かめるテスト。
// 閉じ方の共通規則(背景・Escape・処理中)は dialogs.test.tsx が受け持つ。
import { beforeEach, expect, test, vi } from "vitest";
import { screen, waitFor } from "@testing-library/react";
import * as api from "../api";
import { never } from "../testing/async";
import { renderWithProviders } from "../testing/render";
import type { TagWithCountDto } from "../types";
import { TagBarEditDialog } from "./TagBarEditDialog";

vi.mock("../api", () => ({
  listTagsWithVideoCount: vi.fn(),
  setTagOrder: vi.fn(),
  deleteTag: vi.fn(),
}));

const TAGS: TagWithCountDto[] = [
  { id: 1, name: "猫", video_count: 2 },
  { id: 2, name: "犬", video_count: 1 },
  { id: 3, name: "鳥", video_count: 0 },
];

function names(): (string | null | undefined)[] {
  return screen
    .getAllByTestId("tag-bar-edit-row")
    .map((row) => row.querySelector(".tag-bar-edit-row-name")?.textContent);
}

async function renderDialog(overrides: { onClose?: () => void; onChanged?: () => void } = {}) {
  const onClose = overrides.onClose ?? vi.fn();
  const onChanged = overrides.onChanged ?? vi.fn();
  const view = renderWithProviders(
    <TagBarEditDialog open={true} onClose={onClose} onChanged={onChanged} onTagDeleted={() => {}} />,
  );
  await screen.findAllByTestId("tag-bar-edit-row");
  return { ...view, onClose, onChanged };
}

beforeEach(() => {
  vi.mocked(api.listTagsWithVideoCount).mockResolvedValue(TAGS);
  vi.mocked(api.setTagOrder).mockResolvedValue(undefined);
});

test("↓ を押すと、入れ替えた並びで setTagOrder が呼ばれ、一覧が取り直されて onChanged が呼ばれる", async () => {
  const reordered = [TAGS[1], TAGS[0], TAGS[2]];
  const { user, onChanged } = await renderDialog();
  vi.mocked(api.listTagsWithVideoCount).mockResolvedValue(reordered);

  await user.click(screen.getAllByTestId("tag-bar-edit-move-down-btn")[0]);

  await waitFor(() => expect(onChanged).toHaveBeenCalledTimes(1));
  expect(api.setTagOrder).toHaveBeenCalledWith([2, 1, 3]);
  expect(api.listTagsWithVideoCount).toHaveBeenCalledTimes(2);
  expect(names()).toEqual(["犬", "猫", "鳥"]);
});

test("↑ を押すと、入れ替えた並びで setTagOrder が呼ばれ、表示が取り直した順になる", async () => {
  const { user, onChanged } = await renderDialog();
  vi.mocked(api.listTagsWithVideoCount).mockResolvedValue([TAGS[0], TAGS[2], TAGS[1]]);

  await user.click(screen.getAllByTestId("tag-bar-edit-move-up-btn")[2]);

  await waitFor(() => expect(onChanged).toHaveBeenCalledTimes(1));
  expect(api.setTagOrder).toHaveBeenCalledWith([1, 3, 2]);
  expect(names()).toEqual(["猫", "鳥", "犬"]);
});

test("保存に失敗すると、表示の並びは変わらずエラーが出て、onChanged は呼ばれない", async () => {
  vi.mocked(api.setTagOrder).mockRejectedValue("保存に失敗");
  const { user, onChanged } = await renderDialog();

  await user.click(screen.getAllByTestId("tag-bar-edit-move-down-btn")[0]);

  expect(await screen.findByTestId("tag-bar-edit-error")).toBeInTheDocument();
  expect(names()).toEqual(["猫", "犬", "鳥"]);
  expect(onChanged).not.toHaveBeenCalled();
  expect(api.listTagsWithVideoCount).toHaveBeenCalledTimes(1);
});

test("保存は成功し、取り直しだけ失敗しても、onChanged は呼ばれ、エラーが出る", async () => {
  const { user, onChanged } = await renderDialog();
  vi.mocked(api.listTagsWithVideoCount).mockRejectedValue("取り直しに失敗");

  await user.click(screen.getAllByTestId("tag-bar-edit-move-down-btn")[0]);

  expect(await screen.findByTestId("tag-bar-edit-error")).toBeInTheDocument();
  expect(api.setTagOrder).toHaveBeenCalledWith([2, 1, 3]);
  expect(onChanged).toHaveBeenCalledTimes(1);
});

test("先頭の ↑ と末尾の ↓ は押せず、それ以外は押せる", async () => {
  await renderDialog();

  const ups = screen.getAllByTestId("tag-bar-edit-move-up-btn");
  const downs = screen.getAllByTestId("tag-bar-edit-move-down-btn");
  expect(ups[0]).toBeDisabled();
  expect(ups[1]).toBeEnabled();
  expect(downs[1]).toBeEnabled();
  expect(downs[2]).toBeDisabled();
});

test("並び替えの保存中は、移動・削除・閉じるのボタンが押せない", async () => {
  vi.mocked(api.setTagOrder).mockReturnValue(never());
  const { user } = await renderDialog();

  await user.click(screen.getAllByTestId("tag-bar-edit-move-down-btn")[0]);

  await waitFor(() => expect(screen.getByTestId("tag-bar-edit-close-btn")).toBeDisabled());
  for (const btn of screen.getAllByTestId("tag-bar-edit-move-up-btn")) expect(btn).toBeDisabled();
  for (const btn of screen.getAllByTestId("tag-bar-edit-move-down-btn")) expect(btn).toBeDisabled();
  for (const btn of screen.getAllByTestId("tag-bar-edit-delete-btn")) expect(btn).toBeDisabled();
});

test("「閉じる」で onClose が呼ばれ、保存・キャンセルのボタンは無い", async () => {
  const { user, onClose } = await renderDialog();

  await user.click(screen.getByTestId("tag-bar-edit-close-btn"));

  expect(onClose).toHaveBeenCalledTimes(1);
  expect(screen.queryByTestId("tag-bar-edit-save-btn")).toBeNull();
  expect(screen.queryByTestId("tag-bar-edit-cancel-btn")).toBeNull();
});

test("削除ボタンは aria-label「タグを削除」を持つ", async () => {
  await renderDialog();

  expect(screen.getAllByRole("button", { name: "タグを削除" })).toHaveLength(TAGS.length);
});
