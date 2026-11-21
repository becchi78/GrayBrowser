// TagBarEditDialog の、グループ混在の描画・↑/↓(即保存)・グループの作成/改名/削除・
// タグの削除を確かめるテスト。閉じ方の共通規則(背景・Escape・処理中)は dialogs.test.tsx が受け持つ。
import { beforeEach, expect, test, vi } from "vitest";
import { screen, waitFor, within } from "@testing-library/react";
import * as api from "../api";
import { never } from "../testing/async";
import { renderWithProviders } from "../testing/render";
import type { TagLayoutItemDto } from "../types";
import { TagBarEditDialog } from "./TagBarEditDialog";

vi.mock("../api", () => ({
  getTagLayout: vi.fn(),
  moveTagLayoutItem: vi.fn(),
  createTagGroup: vi.fn(),
  renameTagGroup: vi.fn(),
  deleteTagGroup: vi.fn(),
  deleteTag: vi.fn(),
}));

const tag = (id: number, name: string, video_count = 0): TagLayoutItemDto => ({
  kind: "tag",
  id,
  name,
  video_count,
});
const group = (id: number, name: string, tags: [number, string][] = []): TagLayoutItemDto => ({
  kind: "group",
  id,
  name,
  tags: tags.map(([tid, tname]) => ({ id: tid, name: tname, video_count: 0 })),
});

// 猫 / グループ(鳥, 魚) / 犬
const LAYOUT: TagLayoutItemDto[] = [
  tag(1, "猫", 2),
  group(10, "動物", [
    [3, "鳥"],
    [4, "魚"],
  ]),
  tag(2, "犬", 1),
];

/** 描画順どおりの行(タグは名前、グループは「[名前]」)。 */
function rows(): string[] {
  return screen.getAllByRole("listitem").map((li) => {
    const name = li.querySelector(".tag-bar-edit-row-name")?.textContent ?? "";
    return li.dataset.testid === "tag-bar-edit-group-row" ? `[${name}]` : name;
  });
}

async function renderDialog(
  overrides: { onChanged?: () => void; onTagDeleted?: (id: number) => void } = {},
) {
  const onClose = vi.fn();
  const onChanged = overrides.onChanged ?? vi.fn();
  const onTagDeleted = overrides.onTagDeleted ?? vi.fn();
  const view = renderWithProviders(
    <TagBarEditDialog
      open={true}
      onClose={onClose}
      onChanged={onChanged}
      onTagDeleted={onTagDeleted}
    />,
  );
  await screen.findByTestId("tag-bar-edit-list");
  return { ...view, onClose, onChanged, onTagDeleted };
}

beforeEach(() => {
  vi.mocked(api.getTagLayout).mockResolvedValue(LAYOUT);
  vi.mocked(api.moveTagLayoutItem).mockResolvedValue(LAYOUT);
});

test("トップレベルのタグとグループが並び順どおりに混在し、グループ内の行はインデントのクラスを持つ", async () => {
  await renderDialog();

  expect(rows()).toEqual(["猫", "[動物]", "鳥", "魚", "犬"]);
  const tagRows = screen.getAllByTestId("tag-bar-edit-row");
  expect(tagRows.map((r) => r.classList.contains("tag-bar-edit-row--grouped"))).toEqual([
    false,
    true,
    true,
    false,
  ]);
});

test("タグの ↓ は moveTagLayoutItem('tag', id, 'down') を呼び、返った並びで表示が変わり onChanged が呼ばれる", async () => {
  const { user, onChanged } = await renderDialog();
  vi.mocked(api.moveTagLayoutItem).mockResolvedValue([
    group(10, "動物", [
      [3, "鳥"],
      [4, "魚"],
    ]),
    tag(1, "猫", 2),
    tag(2, "犬", 1),
  ]);

  await user.click(screen.getAllByTestId("tag-bar-edit-move-down-btn")[0]);

  await waitFor(() => expect(onChanged).toHaveBeenCalledTimes(1));
  expect(api.moveTagLayoutItem).toHaveBeenCalledWith("tag", 1, "down");
  expect(rows()).toEqual(["[動物]", "鳥", "魚", "猫", "犬"]);
});

test("グループの ↑ は moveTagLayoutItem('group', id, 'up') を呼び、返った並びで表示が変わる", async () => {
  const { user, onChanged } = await renderDialog();
  vi.mocked(api.moveTagLayoutItem).mockResolvedValue([
    group(10, "動物", [
      [3, "鳥"],
      [4, "魚"],
    ]),
    tag(1, "猫", 2),
    tag(2, "犬", 1),
  ]);

  await user.click(screen.getByTestId("tag-bar-edit-group-move-up-btn"));

  await waitFor(() => expect(onChanged).toHaveBeenCalledTimes(1));
  expect(api.moveTagLayoutItem).toHaveBeenCalledWith("group", 10, "up");
  expect(rows()[0]).toBe("[動物]");
});

test("移動に失敗すると、表示は変わらずエラーが出て、onChanged は呼ばれない", async () => {
  vi.mocked(api.moveTagLayoutItem).mockRejectedValue("保存に失敗");
  const { user, onChanged } = await renderDialog();

  await user.click(screen.getAllByTestId("tag-bar-edit-move-down-btn")[0]);

  expect(await screen.findByTestId("tag-bar-edit-error")).toHaveTextContent("保存に失敗");
  expect(rows()).toEqual(["猫", "[動物]", "鳥", "魚", "犬"]);
  expect(onChanged).not.toHaveBeenCalled();
});

test("↑↓ はトップレベルの先頭の ↑ と末尾の ↓ だけ無効で、グループ内のタグは常に有効", async () => {
  await renderDialog();

  // タグ行は 猫, 鳥, 魚, 犬 の順
  const disabled = (id: string) =>
    screen.getAllByTestId(id).map((b) => (b as HTMLButtonElement).disabled);
  expect(disabled("tag-bar-edit-move-up-btn")).toEqual([true, false, false, false]);
  expect(disabled("tag-bar-edit-move-down-btn")).toEqual([false, false, false, true]);
  expect(screen.getByTestId("tag-bar-edit-group-move-up-btn")).toBeEnabled();
  expect(screen.getByTestId("tag-bar-edit-group-move-down-btn")).toBeEnabled();
});

test("タグとグループの削除ボタンには、ゴミ箱アイコンが出る", async () => {
  await renderDialog();

  for (const id of ["tag-bar-edit-delete-btn", "tag-bar-edit-group-delete-btn"]) {
    for (const btn of screen.getAllByTestId(id)) {
      expect(btn.querySelector("svg")).not.toBeNull();
      expect(btn).not.toHaveTextContent("✕");
    }
  }
});

test("先頭のグループは ↑ が無効", async () => {
  vi.mocked(api.getTagLayout).mockResolvedValue([group(10, "動物"), tag(1, "猫")]);
  await renderDialog();

  expect(screen.getByTestId("tag-bar-edit-group-move-up-btn")).toBeDisabled();
  expect(screen.getByTestId("tag-bar-edit-group-move-down-btn")).toBeEnabled();
});

test("末尾のグループは ↓ が無効", async () => {
  vi.mocked(api.getTagLayout).mockResolvedValue([tag(1, "猫"), group(10, "動物")]);
  await renderDialog();

  expect(screen.getByTestId("tag-bar-edit-group-move-down-btn")).toBeDisabled();
  expect(screen.getByTestId("tag-bar-edit-group-move-up-btn")).toBeEnabled();
});

test("保存中は、移動・編集・削除・追加・閉じるのボタンが押せない", async () => {
  vi.mocked(api.moveTagLayoutItem).mockReturnValue(never());
  const { user } = await renderDialog();

  await user.click(screen.getAllByTestId("tag-bar-edit-move-down-btn")[0]);

  await waitFor(() => expect(screen.getByTestId("tag-bar-edit-close-btn")).toBeDisabled());
  expect(screen.getByTestId("tag-bar-edit-add-group-btn")).toBeDisabled();
  for (const id of [
    "tag-bar-edit-move-up-btn",
    "tag-bar-edit-move-down-btn",
    "tag-bar-edit-delete-btn",
    "tag-bar-edit-group-move-up-btn",
    "tag-bar-edit-group-move-down-btn",
    "tag-bar-edit-group-edit-btn",
    "tag-bar-edit-group-delete-btn",
  ]) {
    for (const btn of screen.getAllByTestId(id)) expect(btn).toBeDisabled();
  }
});

test("「閉じる」で onClose が呼ばれる", async () => {
  const { user, onClose } = await renderDialog();

  await user.click(screen.getByTestId("tag-bar-edit-close-btn"));

  expect(onClose).toHaveBeenCalledTimes(1);
});

test("グループの作成: 入力して「作成」すると、先頭に空のグループが出て、入力行が閉じ onChanged が呼ばれる", async () => {
  vi.mocked(api.createTagGroup).mockResolvedValue([group(11, "新"), ...LAYOUT]);
  const { user, onChanged } = await renderDialog();

  expect(screen.queryByTestId("tag-bar-edit-create")).toBeNull();
  await user.click(screen.getByTestId("tag-bar-edit-add-group-btn"));
  await user.type(screen.getByTestId("tag-bar-edit-create-input"), "新");
  await user.click(screen.getByTestId("tag-bar-edit-create-btn"));

  await waitFor(() => expect(onChanged).toHaveBeenCalledTimes(1));
  expect(api.createTagGroup).toHaveBeenCalledWith("新");
  expect(rows()).toEqual(["[新]", "猫", "[動物]", "鳥", "魚", "犬"]);
  expect(screen.queryByTestId("tag-bar-edit-create")).toBeNull();
});

test("グループの作成に失敗すると、入力行は開いたまま、その場にエラーが出る", async () => {
  vi.mocked(api.createTagGroup).mockRejectedValue("同じ名前のグループがあります。");
  const { user, onChanged } = await renderDialog();

  await user.click(screen.getByTestId("tag-bar-edit-add-group-btn"));
  await user.type(screen.getByTestId("tag-bar-edit-create-input"), "動物");
  await user.click(screen.getByTestId("tag-bar-edit-create-btn"));

  expect(await screen.findByTestId("tag-bar-edit-create-error")).toHaveTextContent(
    "同じ名前のグループがあります。",
  );
  expect(screen.getByTestId("tag-bar-edit-create-input")).toHaveValue("動物");
  expect(onChanged).not.toHaveBeenCalled();
  expect(rows()).toEqual(["猫", "[動物]", "鳥", "魚", "犬"]);
});

test("作成のキャンセルで入力行が閉じ、何も呼ばれない", async () => {
  const { user } = await renderDialog();

  await user.click(screen.getByTestId("tag-bar-edit-add-group-btn"));
  await user.click(screen.getByTestId("tag-bar-edit-create-cancel-btn"));

  expect(screen.queryByTestId("tag-bar-edit-create")).toBeNull();
  expect(api.createTagGroup).not.toHaveBeenCalled();
});

test("グループの改名: ✎ で行内編集になり、保存で renameTagGroup が呼ばれ、新しい名前が出る", async () => {
  vi.mocked(api.renameTagGroup).mockResolvedValue([
    tag(1, "猫", 2),
    group(10, "生き物", [
      [3, "鳥"],
      [4, "魚"],
    ]),
    tag(2, "犬", 1),
  ]);
  const { user, onChanged } = await renderDialog();

  await user.click(screen.getByTestId("tag-bar-edit-group-edit-btn"));
  const input = screen.getByTestId("tag-bar-edit-group-name-input");
  expect(input).toHaveValue("動物");
  await user.clear(input);
  await user.type(input, "生き物");
  await user.click(screen.getByTestId("tag-bar-edit-group-save-btn"));

  await waitFor(() => expect(onChanged).toHaveBeenCalledTimes(1));
  expect(api.renameTagGroup).toHaveBeenCalledWith(10, "生き物");
  expect(screen.queryByTestId("tag-bar-edit-group-name-input")).toBeNull();
  expect(rows()).toEqual(["猫", "[生き物]", "鳥", "魚", "犬"]);
});

test("改名に失敗すると、行内にエラーが出て、編集は開いたまま・保存済みの名前は変わらない", async () => {
  vi.mocked(api.renameTagGroup).mockRejectedValue("グループ名を入力してください。");
  const { user, onChanged } = await renderDialog();

  await user.click(screen.getByTestId("tag-bar-edit-group-edit-btn"));
  await user.clear(screen.getByTestId("tag-bar-edit-group-name-input"));
  await user.click(screen.getByTestId("tag-bar-edit-group-save-btn"));

  expect(await screen.findByTestId("tag-bar-edit-group-edit-error")).toHaveTextContent(
    "グループ名を入力してください。",
  );
  expect(screen.getByTestId("tag-bar-edit-group-name-input")).toBeInTheDocument();
  expect(onChanged).not.toHaveBeenCalled();

  await user.click(screen.getByTestId("tag-bar-edit-group-cancel-btn"));
  expect(rows()).toEqual(["猫", "[動物]", "鳥", "魚", "犬"]);
});

test("グループの削除: 確認文が出て、確定で deleteTagGroup が呼ばれ、返った並びになり onChanged が呼ばれる", async () => {
  vi.mocked(api.deleteTagGroup).mockResolvedValue([
    tag(1, "猫", 2),
    tag(3, "鳥"),
    tag(4, "魚"),
    tag(2, "犬", 1),
  ]);
  const { user, onChanged } = await renderDialog();

  await user.click(screen.getByTestId("tag-bar-edit-group-delete-btn"));
  const confirm = screen.getByTestId("confirm-dialog");
  expect(within(confirm).getByText("タググループを削除")).toBeInTheDocument();
  expect(
    within(confirm).getByText("グループを削除します。中のタグはグループなしに戻ります。"),
  ).toBeInTheDocument();
  await user.click(screen.getByTestId("confirm-dialog-confirm-btn"));

  await waitFor(() => expect(onChanged).toHaveBeenCalledTimes(1));
  expect(api.deleteTagGroup).toHaveBeenCalledWith(10);
  expect(rows()).toEqual(["猫", "鳥", "魚", "犬"]);
  expect(screen.queryByTestId("confirm-dialog")).toBeNull();
});

test("グループの削除の確認をキャンセルすると、何も呼ばれない", async () => {
  const { user } = await renderDialog();

  await user.click(screen.getByTestId("tag-bar-edit-group-delete-btn"));
  await user.click(screen.getByTestId("confirm-dialog-cancel-btn"));

  expect(api.deleteTagGroup).not.toHaveBeenCalled();
  expect(rows()).toHaveLength(5);
});

test("タグの削除: 確定で deleteTag・onTagDeleted・onChanged が呼ばれ、並びが取り直される", async () => {
  vi.mocked(api.deleteTag).mockResolvedValue(undefined);
  const { user, onChanged, onTagDeleted } = await renderDialog();
  vi.mocked(api.getTagLayout).mockResolvedValue([
    tag(1, "猫", 2),
    group(10, "動物", [[4, "魚"]]),
    tag(2, "犬", 1),
  ]);

  // 2番目のタグ行は、グループ内の「鳥」
  await user.click(screen.getAllByTestId("tag-bar-edit-delete-btn")[1]);
  expect(screen.getByTestId("tag-delete-target")).toHaveTextContent("鳥");
  await user.click(screen.getByTestId("confirm-dialog-confirm-btn"));

  await waitFor(() => expect(onChanged).toHaveBeenCalledTimes(1));
  expect(api.deleteTag).toHaveBeenCalledWith(3);
  expect(onTagDeleted).toHaveBeenCalledWith(3);
  expect(api.getTagLayout).toHaveBeenCalledTimes(2);
  await waitFor(() => expect(rows()).toEqual(["猫", "[動物]", "魚", "犬"]));
});

test("閉じて開き直すと、作成・改名の入力状態は残らない", async () => {
  const onClose = vi.fn();
  const props = { onClose, onChanged: vi.fn(), onTagDeleted: vi.fn() };
  const { user, rerender } = renderWithProviders(<TagBarEditDialog open={true} {...props} />);
  await screen.findByTestId("tag-bar-edit-list");

  await user.click(screen.getByTestId("tag-bar-edit-add-group-btn"));
  await user.type(screen.getByTestId("tag-bar-edit-create-input"), "途中");
  await user.click(screen.getByTestId("tag-bar-edit-group-edit-btn"));
  expect(screen.getByTestId("tag-bar-edit-group-name-input")).toBeInTheDocument();

  rerender(<TagBarEditDialog open={false} {...props} />);
  rerender(<TagBarEditDialog open={true} {...props} />);
  await screen.findByTestId("tag-bar-edit-list");

  expect(screen.queryByTestId("tag-bar-edit-create")).toBeNull();
  expect(screen.queryByTestId("tag-bar-edit-group-name-input")).toBeNull();
  await user.click(screen.getByTestId("tag-bar-edit-add-group-btn"));
  expect(screen.getByTestId("tag-bar-edit-create-input")).toHaveValue("");
});

test("開き直したときの取得に失敗すると、前回の一覧は残らずエラーが出る", async () => {
  const props = { onClose: vi.fn(), onChanged: vi.fn(), onTagDeleted: vi.fn() };
  const { rerender } = renderWithProviders(<TagBarEditDialog open={true} {...props} />);
  await screen.findByTestId("tag-bar-edit-list");

  vi.mocked(api.getTagLayout).mockRejectedValue("取得に失敗");
  rerender(<TagBarEditDialog open={false} {...props} />);
  rerender(<TagBarEditDialog open={true} {...props} />);

  expect(await screen.findByTestId("tag-bar-edit-error")).toHaveTextContent("取得に失敗");
  expect(screen.queryByTestId("tag-bar-edit-row")).toBeNull();
});
