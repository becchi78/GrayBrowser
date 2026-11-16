// TagEditor のタグの表示・追加・削除・エラー表示を確かめる。
// タグ名の正規化と検証はバックエンドだけが持つので、ここでは入力をそのまま渡すことと、
// 返ったエラーを出すことを見る。
import { screen, within } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import * as api from "../api";
import { deferred, settle } from "../testing/async";
import { renderWithProviders } from "../testing/render";
import type { TagDto } from "../types";
import { TagEditor } from "./TagEditor";

vi.mock("../api", () => ({
  listTagsForVideo: vi.fn(),
  listAllTags: vi.fn(),
  assignTag: vi.fn(),
  removeTag: vi.fn(),
}));

const CAT: TagDto = { id: 1, name: "猫" };
const DOG: TagDto = { id: 2, name: "犬" };

beforeEach(() => {
  vi.spyOn(console, "error").mockImplementation(() => {});
  vi.mocked(api.listTagsForVideo).mockResolvedValue([CAT]);
  vi.mocked(api.listAllTags).mockResolvedValue([CAT, DOG]);
  vi.mocked(api.assignTag).mockResolvedValue(undefined as never);
  vi.mocked(api.removeTag).mockResolvedValue(undefined as never);
});

async function renderEditor(videoId = "v1") {
  const view = renderWithProviders(<TagEditor videoId={videoId} />);
  await settle();
  return view;
}

const editor = () => screen.getByTestId("video-row-tags");
const input = () => screen.getByPlaceholderText("タグを追加");
const addButton = () => screen.getByRole("button", { name: "追加" });
const chipNames = () =>
  Array.from(editor().querySelectorAll(".tag-chip")).map((c) => c.firstChild?.textContent);

test("動画に付いているタグをチップで出し、既存のタグを入力の候補にする", async () => {
  await renderEditor();

  expect(chipNames()).toEqual(["猫"]);
  expect(api.listTagsForVideo).toHaveBeenCalledWith("v1");
  const options = Array.from(editor().querySelectorAll("datalist option")).map(
    (o) => (o as HTMLOptionElement).value,
  );
  expect(options).toEqual(["猫", "犬"]);
});

test("タグを追加すると、入力そのままを渡し、入力を空にして、タグを取り直す", async () => {
  const { user } = await renderEditor();
  vi.mocked(api.listTagsForVideo).mockResolvedValue([CAT, DOG]);

  await user.type(input(), " 犬 ");
  await user.click(addButton());

  expect(api.assignTag).toHaveBeenCalledWith("v1", " 犬 ");
  expect(input()).toHaveValue("");
  expect(chipNames()).toEqual(["猫", "犬"]);
});

test("Enter でも追加できる", async () => {
  const { user } = await renderEditor();

  await user.type(input(), "犬{Enter}");

  expect(api.assignTag).toHaveBeenCalledWith("v1", "犬");
});

test("入力が空か空白だけなら、追加を呼ばない", async () => {
  const { user } = await renderEditor();

  await user.click(addButton());
  await user.type(input(), "   ");
  await user.click(addButton());

  expect(api.assignTag).not.toHaveBeenCalled();
});

test("追加に失敗すると、エラーを出し、入力を残し、タグを取り直さない", async () => {
  vi.mocked(api.assignTag).mockRejectedValue(new Error("この動画は見つかりません"));
  const { user } = await renderEditor();
  vi.mocked(api.listTagsForVideo).mockClear();

  await user.type(input(), "犬");
  await user.click(addButton());

  expect(editor()).toHaveTextContent("この動画は見つかりません");
  expect(input()).toHaveValue("犬");
  expect(api.listTagsForVideo).not.toHaveBeenCalled();
});

test("次の操作を始めると、前のエラーは消える", async () => {
  vi.mocked(api.assignTag).mockRejectedValueOnce(new Error("この動画は見つかりません"));
  const { user } = await renderEditor();
  await user.type(input(), "犬");
  await user.click(addButton());
  expect(editor()).toHaveTextContent("この動画は見つかりません");

  await user.click(addButton());

  expect(editor()).not.toHaveTextContent("この動画は見つかりません");
});

test("追加できたあとのタグの取り直しに失敗しても、追加のエラーとしては出さない", async () => {
  const { user } = await renderEditor();
  vi.mocked(api.listTagsForVideo).mockRejectedValue(new Error("取り直せない"));

  await user.type(input(), "犬");
  await user.click(addButton());

  expect(api.assignTag).toHaveBeenCalledWith("v1", "犬");
  expect(input()).toHaveValue("");
  expect(editor()).not.toHaveTextContent("取り直せない");
});

test("チップの × でタグを外し、タグを取り直す", async () => {
  const { user } = await renderEditor();
  vi.mocked(api.listTagsForVideo).mockResolvedValue([]);

  await user.click(screen.getByRole("button", { name: "猫を削除" }));

  expect(api.removeTag).toHaveBeenCalledWith("v1", 1);
  expect(chipNames()).toEqual([]);
});

test("外すのに失敗すると、エラーを出し、チップを残す", async () => {
  vi.mocked(api.removeTag).mockRejectedValue(new Error("外せない"));
  const { user } = await renderEditor();

  await user.click(screen.getByRole("button", { name: "猫を削除" }));

  expect(editor()).toHaveTextContent("外せない");
  expect(chipNames()).toEqual(["猫"]);
});

test("最初のタグの取得に失敗しても落ちず、タグを追加できる", async () => {
  vi.mocked(api.listTagsForVideo).mockRejectedValueOnce(new Error("取れない"));
  const { user } = await renderEditor();
  expect(chipNames()).toEqual([]);

  await user.type(input(), "犬");
  await user.click(addButton());

  expect(api.assignTag).toHaveBeenCalledWith("v1", "犬");
  expect(chipNames()).toEqual(["猫"]);
});

test("tagsVersion が変わると、タグを取り直す", async () => {
  const { rerender } = await renderEditor();
  vi.mocked(api.listTagsForVideo).mockResolvedValue([CAT, DOG]);

  rerender(<TagEditor videoId="v1" tagsVersion={1} />);
  await settle();

  expect(chipNames()).toEqual(["猫", "犬"]);
});

test("動画が変わった後に、前の動画のタグの応答が遅れて届いても、表示を上書きしない", async () => {
  const slow = deferred<TagDto[]>();
  vi.mocked(api.listTagsForVideo).mockReturnValueOnce(slow.promise);
  const { rerender } = renderWithProviders(<TagEditor videoId="old" />);
  vi.mocked(api.listTagsForVideo).mockResolvedValueOnce([DOG]);
  rerender(<TagEditor videoId="new" />);
  await settle();
  expect(chipNames()).toEqual(["犬"]);

  slow.resolve([CAT]);
  await settle();

  expect(chipNames()).toEqual(["犬"]);
  expect(within(editor()).queryByText("猫")).toBeNull();
});

test("外すのに成功すると、前の操作のエラーは消える", async () => {
  vi.mocked(api.removeTag).mockRejectedValueOnce(new Error("外せない"));
  const { user } = await renderEditor();
  await user.click(screen.getByRole("button", { name: "猫を削除" }));
  expect(editor()).toHaveTextContent("外せない");
  vi.mocked(api.listTagsForVideo).mockResolvedValue([]);

  await user.click(screen.getByRole("button", { name: "猫を削除" }));

  expect(editor()).not.toHaveTextContent("外せない");
  expect(chipNames()).toEqual([]);
});
