// BulkTagDialog のタグの複数選択と、タグの付与・解除の成否を確かめる。
import { fireEvent, screen } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import * as api from "../api";
import { deferred } from "../testing/async";
import { renderWithProviders } from "../testing/render";
import { BulkTagDialog } from "./BulkTagDialog";

vi.mock("../api", () => ({
  listAllTags: vi.fn(),
  assignTagsToVideos: vi.fn(),
  listTagUsageForVideos: vi.fn(),
  removeTagsFromVideos: vi.fn(),
}));

beforeEach(() => {
  vi.spyOn(console, "error").mockImplementation(() => {});
  vi.mocked(api.listAllTags).mockResolvedValue([
    { id: 1, name: "猫" },
    { id: 2, name: "犬" },
    { id: 3, name: "子猫" },
  ]);
  vi.mocked(api.assignTagsToVideos).mockResolvedValue(undefined as never);
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
const pressedNames = () =>
  Array.from(screen.getByTestId("bulk-tag-list").querySelectorAll("button"))
    .filter((b) => b.getAttribute("aria-pressed") === "true")
    .map((b) => b.textContent);
const tagButton = (name: string) => screen.getByRole("button", { name });

test("開くと既存のタグを候補に並べ、件数を見出しに出す", async () => {
  await renderDialog();

  expect(candidateNames()).toEqual(["猫", "犬", "子猫"]);
  expect(screen.getByRole("heading")).toHaveTextContent("2 件の動画にタグを付ける");
  expect(input()).toHaveFocus();
});

test("入力欄は新しいタグ名用で、表示は「タグ名」だけにする", async () => {
  await renderDialog();

  expect(input()).toHaveAttribute("placeholder", "タグ名");
  expect(input()).toHaveAccessibleName("タグ名");
});

test("入力した文字に部分一致するタグだけに、一覧が絞り込まれる", async () => {
  const { user } = await renderDialog();

  await user.type(input(), "猫");
  expect(candidateNames()).toEqual(["猫", "子猫"]);

  await user.clear(input());
  expect(candidateNames()).toEqual(["猫", "犬", "子猫"]);
});

test("絞り込みは大文字小文字を区別しない", async () => {
  vi.mocked(api.listAllTags).mockResolvedValue([
    { id: 1, name: "Action" },
    { id: 2, name: "drama" },
  ]);
  const { user } = renderWithProviders(
    <BulkTagDialog videoIds={["a"]} onTagged={vi.fn()} onClose={vi.fn()} />,
  );
  await screen.findByRole("button", { name: "drama" });

  await user.type(input(), "ACTION");
  expect(candidateNames()).toEqual(["Action"]);
});

test("選んだタグは、絞り込みで外れても一覧に残り、並びも元のまま", async () => {
  const { user } = await renderDialog();
  await user.click(tagButton("犬"));

  await user.type(input(), "猫");
  expect(candidateNames()).toEqual(["猫", "犬", "子猫"]);
  expect(pressedNames()).toEqual(["犬"]);
});

test("入力を変えても選択は保たれ、送信に含まれる", async () => {
  const { user, onTagged } = await renderDialog();
  await user.click(tagButton("犬"));
  await user.type(input(), "子");
  expect(candidateNames()).toEqual(["犬", "子猫"]);
  await user.click(tagButton("子猫"));
  expect(input()).toHaveValue("子");

  await user.clear(input());
  expect(pressedNames()).toEqual(["犬", "子猫"]);

  await user.click(submit());
  expect(api.assignTagsToVideos).toHaveBeenCalledWith(["a", "b"], ["犬", "子猫"]);
  expect(onTagged).toHaveBeenCalled();
});

test("絞り込みが0件でも、入力した名前を新しいタグとして付けられる", async () => {
  const { user } = await renderDialog();

  await user.type(input(), "新規");
  expect(candidateNames()).toEqual([]);
  expect(submit()).toBeEnabled();

  await user.click(submit());
  expect(api.assignTagsToVideos).toHaveBeenCalledWith(["a", "b"], ["新規"]);
});

test("候補を押すと選択を切り替え、選んだものは aria-pressed で伝わり、入力は変わらない", async () => {
  const { user } = await renderDialog();
  expect(pressedNames()).toEqual([]);

  await user.click(tagButton("犬"));
  expect(tagButton("犬")).toHaveAttribute("aria-pressed", "true");
  expect(tagButton("猫")).toHaveAttribute("aria-pressed", "false");
  expect(input()).toHaveValue("");

  await user.click(tagButton("子猫"));
  expect(pressedNames()).toEqual(["犬", "子猫"]);

  await user.click(tagButton("犬"));
  expect(pressedNames()).toEqual(["子猫"]);
  expect(candidateNames()).toEqual(["猫", "犬", "子猫"]);
});

test("選択が無く入力も空か空白だけの間は、OK を押せない", async () => {
  const { user } = await renderDialog();
  expect(submit()).toBeDisabled();

  await user.type(input(), "   ");
  expect(submit()).toBeDisabled();

  await user.type(input(), "猫");
  expect(submit()).toBeEnabled();
});

test("入力が空でも、タグを1つ選べば OK を押せ、選びを外せばまた押せない", async () => {
  const { user } = await renderDialog();

  await user.click(tagButton("猫"));
  expect(submit()).toBeEnabled();

  await user.click(tagButton("猫"));
  expect(submit()).toBeDisabled();
});

test("付与に成功すると、対象の動画と入力そのままのタグ名を渡し、通知してから閉じる", async () => {
  const order: string[] = [];
  const { user, onTagged, onClose } = await renderDialog();
  onTagged.mockImplementation(() => order.push("tagged"));
  onClose.mockImplementation(() => order.push("close"));
  await user.type(input(), " 新しいタグ ");

  await user.click(submit());

  expect(api.assignTagsToVideos).toHaveBeenCalledWith(["a", "b"], [" 新しいタグ "]);
  expect(order).toEqual(["tagged", "close"]);
});

test("選んだ既存のタグだけを、複数まとめて付与できる(一覧の並び順で渡す)", async () => {
  const { user, onTagged } = await renderDialog();
  await user.click(tagButton("子猫"));
  await user.click(tagButton("猫"));

  await user.click(submit());

  expect(api.assignTagsToVideos).toHaveBeenCalledWith(["a", "b"], ["猫", "子猫"]);
  expect(onTagged).toHaveBeenCalledTimes(1);
});

test("選んだ既存のタグに、入力欄の新しい名前を足して付与する", async () => {
  const { user } = await renderDialog();
  await user.click(tagButton("犬"));
  await user.type(input(), "新しいタグ");

  await user.click(submit());

  expect(api.assignTagsToVideos).toHaveBeenCalledWith(["a", "b"], ["犬", "新しいタグ"]);
});

test("付与に失敗すると、エラーを出し、通知も閉じる操作もせず、入力を残して再実行できる", async () => {
  vi.mocked(api.assignTagsToVideos).mockRejectedValueOnce(new Error("タグ名が長すぎます"));
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
  vi.mocked(api.assignTagsToVideos).mockReturnValue(pending.promise);
  const { user } = await renderDialog();
  await user.click(tagButton("犬"));
  await user.type(input(), "猫");

  await user.click(submit());

  expect(submit()).toHaveTextContent("処理中...");
  expect(submit()).toBeDisabled();
  expect(input()).toBeDisabled();
  expect(screen.getByTestId("bulk-tag-cancel-btn")).toBeDisabled();
  expect(tagButton("子猫")).toBeDisabled();
  // 処理中も選択は見えたまま
  expect(pressedNames()).toEqual(["犬"]);
});

test("既存のタグの一覧を取得できなくても、入力して付与できる", async () => {
  vi.mocked(api.listAllTags).mockRejectedValue(new Error("取れない"));
  const { user, onClose } = renderWithProvidersWithoutTags();
  await user.type(input(), "猫");

  await user.click(submit());

  expect(api.assignTagsToVideos).toHaveBeenCalledWith(["a"], ["猫"]);
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

test("付与の実行中は、Escape でも背景のクリックでも閉じず、再送信もされない", async () => {
  const pending = deferred<never>();
  vi.mocked(api.assignTagsToVideos).mockReturnValue(pending.promise);
  const { user, onClose } = await renderDialog();
  await user.type(input(), "猫");
  await user.click(submit());
  expect(submit()).toBeDisabled();

  await user.keyboard("{Escape}");
  await user.click(screen.getByTestId("bulk-tag-dialog").parentElement!);
  fireEvent.submit(input().closest("form")!);

  expect(onClose).not.toHaveBeenCalled();
  expect(api.assignTagsToVideos).toHaveBeenCalledTimes(1);
});

// ---- 「外す」タブ ----

const usage = [
  { id: 1, name: "猫", video_count: 2 },
  { id: 2, name: "犬", video_count: 1 },
];

async function renderRemoveTab() {
  const onTagged = vi.fn();
  const onClose = vi.fn();
  const view = renderWithProviders(
    <BulkTagDialog videoIds={["a", "b"]} onTagged={onTagged} onClose={onClose} />,
  );
  await view.user.click(screen.getByRole("tab", { name: "外す" }));
  return { ...view, onTagged, onClose };
}

const removeBtn = () => screen.getByTestId("bulk-tag-remove-btn");

beforeEach(() => {
  vi.mocked(api.listTagUsageForVideos).mockResolvedValue(usage);
  vi.mocked(api.removeTagsFromVideos).mockResolvedValue(undefined as never);
});

test("タブは「付ける」が選ばれた状態で開き、切り替えると選択と表示が入れ替わる", async () => {
  const { user } = await renderDialog();
  expect(screen.getByRole("tablist")).toBeInTheDocument();
  expect(screen.getByRole("tab", { name: "付ける" })).toHaveAttribute("aria-selected", "true");
  expect(screen.getByRole("tab", { name: "外す" })).toHaveAttribute("aria-selected", "false");
  expect(api.listTagUsageForVideos).not.toHaveBeenCalled();

  await user.click(screen.getByRole("tab", { name: "外す" }));

  expect(screen.getByRole("tab", { name: "外す" })).toHaveAttribute("aria-selected", "true");
  expect(screen.getByRole("tab", { name: "付ける" })).toHaveAttribute("aria-selected", "false");
  expect(await screen.findByRole("tabpanel", { name: "外す" })).toBeInTheDocument();
  expect(api.listTagUsageForVideos).toHaveBeenCalledWith(["a", "b"]);
  expect(screen.getByRole("heading")).toHaveTextContent("2 件の動画にタグを付ける");
});

test("付けるに戻ると、入力していた内容が残っている", async () => {
  const { user } = await renderDialog();
  await user.type(input(), "猫");

  await user.click(screen.getByRole("tab", { name: "外す" }));
  await user.click(screen.getByRole("tab", { name: "付ける" }));

  expect(input()).toHaveValue("猫");
});

test("外すタブは、付いているタグを「タグ名(n/N 件)」の複数選択の一覧で並べ、選ぶまで外すを押せない", async () => {
  const { user } = await renderRemoveTab();

  const cat = await screen.findByRole("button", { name: "猫(2/2 件)" });
  const dog = screen.getByRole("button", { name: "犬(1/2 件)" });
  expect(cat).toHaveAttribute("aria-pressed", "false");
  expect(removeBtn()).toBeDisabled();

  await user.click(dog);
  expect(dog).toHaveAttribute("aria-pressed", "true");
  expect(removeBtn()).toBeEnabled();

  await user.click(cat);
  expect(cat).toHaveAttribute("aria-pressed", "true");
  expect(dog).toHaveAttribute("aria-pressed", "true");

  await user.click(dog);
  await user.click(cat);
  expect(removeBtn()).toBeDisabled();
});

test("タブを切り替えて戻ると、外すの選択は空に戻る", async () => {
  const { user } = await renderRemoveTab();
  await user.click(await screen.findByRole("button", { name: "犬(1/2 件)" }));

  await user.click(screen.getByRole("tab", { name: "付ける" }));
  await user.click(screen.getByRole("tab", { name: "外す" }));

  expect(await screen.findByRole("button", { name: "犬(1/2 件)" })).toHaveAttribute(
    "aria-pressed",
    "false",
  );
  expect(removeBtn()).toBeDisabled();
});

test("選んで外すを押すと、選んだタグ名を並べた確認を出し、キャンセルすれば何もしない", async () => {
  const { user, onTagged, onClose } = await renderRemoveTab();
  await user.click(await screen.findByRole("button", { name: "犬(1/2 件)" }));
  await user.click(screen.getByRole("button", { name: "猫(2/2 件)" }));

  await user.click(removeBtn());

  expect(screen.getByTestId("confirm-dialog")).toHaveTextContent(
    "『猫』『犬』を、付いている動画から外します。",
  );
  expect(api.removeTagsFromVideos).not.toHaveBeenCalled();

  await user.click(screen.getByTestId("confirm-dialog-cancel-btn"));

  expect(screen.queryByTestId("confirm-dialog")).toBeNull();
  expect(api.removeTagsFromVideos).not.toHaveBeenCalled();
  expect(onTagged).not.toHaveBeenCalled();
  expect(onClose).not.toHaveBeenCalled();
  expect(screen.getByTestId("bulk-tag-dialog")).toBeInTheDocument();
});

test("1つだけ選んだときの確認は、そのタグ名だけを出す", async () => {
  const { user } = await renderRemoveTab();
  await user.click(await screen.findByRole("button", { name: "犬(1/2 件)" }));

  await user.click(removeBtn());

  expect(screen.getByTestId("confirm-dialog")).toHaveTextContent(
    "『犬』を、付いている動画から外します。",
  );
});

test("選んだタグが多いときの確認は、タグ名を並べず個数にする", async () => {
  vi.mocked(api.listTagUsageForVideos).mockResolvedValue([
    { id: 1, name: "あ", video_count: 2 },
    { id: 2, name: "い", video_count: 2 },
    { id: 3, name: "う", video_count: 1 },
    { id: 4, name: "え", video_count: 1 },
  ]);
  const { user } = await renderRemoveTab();
  for (const name of ["あ(2/2 件)", "い(2/2 件)", "う(1/2 件)"]) {
    await user.click(await screen.findByRole("button", { name }));
  }
  await user.click(removeBtn());
  expect(screen.getByTestId("confirm-dialog")).toHaveTextContent(
    "『あ』『い』『う』を、付いている動画から外します。",
  );
  await user.click(screen.getByTestId("confirm-dialog-cancel-btn"));

  await user.click(screen.getByRole("button", { name: "え(1/2 件)" }));
  await user.click(removeBtn());

  expect(screen.getByTestId("confirm-dialog")).toHaveTextContent(
    "選んだ 4 個のタグを、付いている動画から外します。",
  );
});

test("確認を受けると、対象の動画と選んだタグの ID を渡し、通知してから閉じる", async () => {
  const order: string[] = [];
  const { user, onTagged, onClose } = await renderRemoveTab();
  onTagged.mockImplementation(() => order.push("tagged"));
  onClose.mockImplementation(() => order.push("close"));
  await user.click(await screen.findByRole("button", { name: "犬(1/2 件)" }));
  await user.click(screen.getByRole("button", { name: "猫(2/2 件)" }));
  await user.click(removeBtn());

  await user.click(screen.getByTestId("confirm-dialog-confirm-btn"));

  expect(api.removeTagsFromVideos).toHaveBeenCalledWith(["a", "b"], [1, 2]);
  expect(order).toEqual(["tagged", "close"]);
});

test("外すのに失敗すると、確認を閉じてエラーを出し、通知も閉じる操作もせず、やり直せる", async () => {
  vi.mocked(api.removeTagsFromVideos).mockRejectedValueOnce(new Error("書けません"));
  const { user, onTagged, onClose } = await renderRemoveTab();
  await user.click(await screen.findByRole("button", { name: "猫(2/2 件)" }));
  await user.click(removeBtn());

  await user.click(screen.getByTestId("confirm-dialog-confirm-btn"));

  expect(await screen.findByTestId("bulk-tag-remove-error")).toHaveTextContent("書けません");
  expect(screen.queryByTestId("confirm-dialog")).toBeNull();
  expect(onTagged).not.toHaveBeenCalled();
  expect(onClose).not.toHaveBeenCalled();
  expect(removeBtn()).toBeEnabled();

  await user.click(removeBtn());
  await user.click(screen.getByTestId("confirm-dialog-confirm-btn"));

  expect(onTagged).toHaveBeenCalledTimes(1);
  expect(onClose).toHaveBeenCalledTimes(1);
});

test("外す処理中は、確認・タブ・選択・キャンセルを無効にし、Escape でも閉じない", async () => {
  const pending = deferred<never>();
  vi.mocked(api.removeTagsFromVideos).mockReturnValue(pending.promise);
  const { user, onClose } = await renderRemoveTab();
  await user.click(await screen.findByRole("button", { name: "猫(2/2 件)" }));
  await user.click(removeBtn());

  await user.click(screen.getByTestId("confirm-dialog-confirm-btn"));

  expect(screen.getByTestId("confirm-dialog-confirm-btn")).toBeDisabled();
  expect(screen.getByTestId("confirm-dialog-confirm-btn")).toHaveTextContent("処理中...");
  expect(screen.getByTestId("confirm-dialog-cancel-btn")).toBeDisabled();
  expect(screen.getByRole("tab", { name: "付ける" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "犬(1/2 件)" })).toBeDisabled();
  expect(screen.getByTestId("bulk-tag-remove-cancel-btn")).toBeDisabled();

  await user.keyboard("{Escape}");
  expect(onClose).not.toHaveBeenCalled();
  expect(api.removeTagsFromVideos).toHaveBeenCalledTimes(1);
});

test("確認が開いている間の Escape は確認だけを閉じ、ダイアログは残る", async () => {
  const { user, onClose } = await renderRemoveTab();
  await user.click(await screen.findByRole("button", { name: "猫(2/2 件)" }));
  await user.click(removeBtn());

  await user.keyboard("{Escape}");

  expect(screen.queryByTestId("confirm-dialog")).toBeNull();
  expect(onClose).not.toHaveBeenCalled();
});

test("付いているタグが無いときは、その旨を出し、外すを押せない", async () => {
  vi.mocked(api.listTagUsageForVideos).mockResolvedValue([]);
  await renderRemoveTab();

  expect(await screen.findByTestId("bulk-tag-usage-empty")).toHaveTextContent(
    "選択した動画にタグは付いていません。",
  );
  expect(removeBtn()).toBeDisabled();
});

test("付いているタグの取得に失敗すると、エラーを出し、外すを押せない", async () => {
  vi.mocked(api.listTagUsageForVideos).mockRejectedValue(new Error("取れない"));
  await renderRemoveTab();

  expect(await screen.findByTestId("bulk-tag-usage-error")).toHaveTextContent("取れない");
  expect(removeBtn()).toBeDisabled();
});
