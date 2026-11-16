// BulkTagDialog の候補の選択と、タグの付与の成否を確かめる。
// 候補の絞り込み規則は lib/bulkTag のテストが持つので、ここでは画面への出方を見る。
import { screen } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import * as api from "../api";
import { deferred } from "../testing/async";
import { renderWithProviders } from "../testing/render";
import { BulkTagDialog } from "./BulkTagDialog";

vi.mock("../api", () => ({
  listAllTags: vi.fn(),
  assignTagToVideos: vi.fn(),
}));

beforeEach(() => {
  vi.spyOn(console, "error").mockImplementation(() => {});
  vi.mocked(api.listAllTags).mockResolvedValue([
    { id: 1, name: "猫" },
    { id: 2, name: "犬" },
    { id: 3, name: "子猫" },
  ]);
  vi.mocked(api.assignTagToVideos).mockResolvedValue(undefined as never);
});

async function renderDialog() {
  const onTagged = vi.fn();
  const onClose = vi.fn();
  const view = renderWithProviders(
    <BulkTagDialog videoIds={["a", "b"]} onTagged={onTagged} onClose={onClose} />,
  );
  await screen.findByRole("button", { name: "犬" });
  return { ...view, onTagged, onClose };
}

const input = () => screen.getByTestId("bulk-tag-input");
const submit = () => screen.getByTestId("bulk-tag-submit-btn");
const candidateNames = () =>
  Array.from(screen.getByTestId("bulk-tag-list").querySelectorAll("button")).map(
    (b) => b.textContent,
  );

test("開くと既存のタグを候補に並べ、件数を見出しに出す", async () => {
  await renderDialog();

  expect(candidateNames()).toEqual(["猫", "犬", "子猫"]);
  expect(screen.getByRole("heading")).toHaveTextContent("2 件の動画にタグを付ける");
  expect(input()).toHaveFocus();
});

test("入力に合う候補だけに絞り、合うものが無ければ候補の欄を出さない", async () => {
  const { user } = await renderDialog();

  await user.type(input(), "猫");
  expect(candidateNames()).toEqual(["猫", "子猫"]);

  await user.type(input(), "ABC");
  expect(screen.queryByTestId("bulk-tag-list")).toBeNull();
});

test("候補を選ぶと入力に反映し、入力へフォーカスを戻す", async () => {
  const { user } = await renderDialog();

  await user.click(screen.getByRole("button", { name: "犬" }));

  expect(input()).toHaveValue("犬");
  expect(input()).toHaveFocus();
});

test("入力が空か空白だけの間は、OK を押せない", async () => {
  const { user } = await renderDialog();
  expect(submit()).toBeDisabled();

  await user.type(input(), "   ");
  expect(submit()).toBeDisabled();

  await user.type(input(), "猫");
  expect(submit()).toBeEnabled();
});

test("付与に成功すると、対象の動画と入力そのままのタグ名を渡し、通知してから閉じる", async () => {
  const order: string[] = [];
  const { user, onTagged, onClose } = await renderDialog();
  onTagged.mockImplementation(() => order.push("tagged"));
  onClose.mockImplementation(() => order.push("close"));
  await user.type(input(), " 新しいタグ ");

  await user.click(submit());

  expect(api.assignTagToVideos).toHaveBeenCalledWith(["a", "b"], " 新しいタグ ");
  expect(order).toEqual(["tagged", "close"]);
});

test("候補から選んだタグも、そのまま付与できる", async () => {
  const { user, onTagged } = await renderDialog();
  await user.click(screen.getByRole("button", { name: "子猫" }));

  await user.click(submit());

  expect(api.assignTagToVideos).toHaveBeenCalledWith(["a", "b"], "子猫");
  expect(onTagged).toHaveBeenCalledTimes(1);
});

test("付与に失敗すると、エラーを出し、通知も閉じる操作もせず、入力を残して再実行できる", async () => {
  vi.mocked(api.assignTagToVideos).mockRejectedValueOnce(new Error("タグ名が長すぎます"));
  const { user, onTagged, onClose } = await renderDialog();
  await user.type(input(), "猫");

  await user.click(submit());

  expect(await screen.findByTestId("bulk-tag-error")).toHaveTextContent("タグ名が長すぎます");
  expect(onTagged).not.toHaveBeenCalled();
  expect(onClose).not.toHaveBeenCalled();
  expect(input()).toHaveValue("猫");
  expect(submit()).toBeEnabled();

  await user.click(submit());

  expect(onTagged).toHaveBeenCalledTimes(1);
  expect(onClose).toHaveBeenCalledTimes(1);
  expect(screen.queryByTestId("bulk-tag-error")).toBeNull();
});

test("付与中は「処理中...」を出し、入力・候補・ボタンを無効にする", async () => {
  const pending = deferred<never>();
  vi.mocked(api.assignTagToVideos).mockReturnValue(pending.promise);
  const { user } = await renderDialog();
  await user.type(input(), "猫");

  await user.click(submit());

  expect(submit()).toHaveTextContent("処理中...");
  expect(submit()).toBeDisabled();
  expect(input()).toBeDisabled();
  expect(screen.getByTestId("bulk-tag-cancel-btn")).toBeDisabled();
  expect(screen.getByRole("button", { name: "子猫" })).toBeDisabled();
});

test("既存のタグの一覧を取得できなくても、入力して付与できる", async () => {
  vi.mocked(api.listAllTags).mockRejectedValue(new Error("取れない"));
  const { user, onClose } = renderWithProvidersWithoutTags();
  await user.type(input(), "猫");

  await user.click(submit());

  expect(api.assignTagToVideos).toHaveBeenCalledWith(["a"], "猫");
  expect(onClose).toHaveBeenCalledTimes(1);
  expect(screen.queryByTestId("bulk-tag-list")).toBeNull();
});

function renderWithProvidersWithoutTags() {
  const onClose = vi.fn();
  const view = renderWithProviders(
    <BulkTagDialog videoIds={["a"]} onTagged={vi.fn()} onClose={onClose} />,
  );
  return { ...view, onClose };
}
