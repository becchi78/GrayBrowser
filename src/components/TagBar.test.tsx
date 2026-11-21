// TagBar の描画・グループの開閉と折りたたみ状態の保存・絞り込みの選択を確かめる。
// グループは表示上の入れ物なので、選択(selected / onChange)の扱いが
// グループの有無で変わらないことを見る。
import { screen } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import * as api from "../api";
import { settle } from "../testing/async";
import { renderWithProviders } from "../testing/render";
import type { TagLayoutItemDto } from "../types";
import { TagBar } from "./TagBar";

vi.mock("../api", () => ({
  getTagLayout: vi.fn(),
  getCollapsedTagGroups: vi.fn(),
  setCollapsedTagGroups: vi.fn(),
}));

vi.mock("../events", () => ({
  onCatalogChanged: vi.fn(() => () => {}),
}));

const tag = (id: number, name: string): TagLayoutItemDto => ({
  kind: "tag",
  id,
  name,
  video_count: 1,
});
const group = (id: number, name: string, tags: [number, string][] = []): TagLayoutItemDto => ({
  kind: "group",
  id,
  name,
  tags: tags.map(([tid, tname]) => ({ id: tid, name: tname, video_count: 1 })),
});

// 猫(グループなし)、動物グループ(犬・鳥)、空のグループ
const LAYOUT: TagLayoutItemDto[] = [
  tag(1, "猫"),
  group(10, "動物", [
    [2, "犬"],
    [3, "鳥"],
  ]),
  group(11, "空"),
];

beforeEach(() => {
  vi.spyOn(console, "error").mockImplementation(() => {});
  vi.mocked(api.getTagLayout).mockResolvedValue(LAYOUT);
  vi.mocked(api.getCollapsedTagGroups).mockResolvedValue([]);
  vi.mocked(api.setCollapsedTagGroups).mockResolvedValue(undefined);
});

async function renderBar(selected: number[] = [], onChange = vi.fn()) {
  const view = renderWithProviders(<TagBar selected={selected} onChange={onChange} />);
  await settle();
  return { ...view, onChange };
}

const chips = () => screen.queryAllByTestId("tag-bar-chip");
const chipNames = () => chips().map((c) => c.textContent);
const groups = () => screen.queryAllByTestId("tag-bar-group");

test("グループがないときは、タグのボタンだけを従来の形で並べる", async () => {
  vi.mocked(api.getTagLayout).mockResolvedValue([tag(1, "猫"), tag(2, "犬")]);
  await renderBar([2]);

  expect(chipNames()).toEqual(["猫", "犬"]);
  expect(groups()).toHaveLength(0);
  expect(chips()[0]).toHaveAttribute("aria-pressed", "false");
  expect(chips()[1]).toHaveAttribute("aria-pressed", "true");
  expect(chips()[0]).toHaveAttribute("title", "猫");
  expect(chips()[0]).toHaveClass("folder-sidebar-item");
  expect(chips()[0]).not.toHaveClass("tag-bar-chip--grouped");
  expect(screen.getByTestId("tag-bar-clear-all")).toBeEnabled();
});

test("グループを見出しとして出し、既定では開いて中のタグを字下げして並べる", async () => {
  await renderBar();

  expect(groups().map((g) => g.textContent)).toEqual(["動物", "空"]);
  expect(groups()[0]).toHaveAttribute("aria-expanded", "true");
  expect(chipNames()).toEqual(["猫", "犬", "鳥"]);
  expect(chips()[0]).not.toHaveClass("tag-bar-chip--grouped");
  expect(chips()[1]).toHaveClass("tag-bar-chip--grouped");
  expect(chips()[2]).toHaveClass("tag-bar-chip--grouped");
});

test("グループ行のクリックで開閉し、閉じたグループの中のタグは描画せず、状態を保存する", async () => {
  const { user, onChange } = await renderBar();

  await user.click(groups()[0]);
  expect(groups()[0]).toHaveAttribute("aria-expanded", "false");
  expect(chipNames()).toEqual(["猫"]);
  expect(api.setCollapsedTagGroups).toHaveBeenLastCalledWith([10]);

  await user.click(groups()[0]);
  expect(groups()[0]).toHaveAttribute("aria-expanded", "true");
  expect(chipNames()).toEqual(["猫", "犬", "鳥"]);
  expect(api.setCollapsedTagGroups).toHaveBeenLastCalledWith([]);

  // 開閉は選択に触れない。
  expect(onChange).not.toHaveBeenCalled();
});

test("保存済みの折りたたみ状態を復元する", async () => {
  vi.mocked(api.getCollapsedTagGroups).mockResolvedValue([10]);
  await renderBar();

  expect(groups()[0]).toHaveAttribute("aria-expanded", "false");
  expect(groups()[1]).toHaveAttribute("aria-expanded", "true");
  expect(chipNames()).toEqual(["猫"]);
});

test("保存時に、存在しないグループの id は落とす", async () => {
  vi.mocked(api.getCollapsedTagGroups).mockResolvedValue([999, 10]);
  const { user } = await renderBar();

  await user.click(groups()[1]);
  expect(api.setCollapsedTagGroups).toHaveBeenLastCalledWith([10, 11]);

  await user.click(groups()[1]);
  expect(api.setCollapsedTagGroups).toHaveBeenLastCalledWith([10]);
});

test("折りたたみ状態の保存に失敗しても、表示は切り替わったままにする", async () => {
  vi.mocked(api.setCollapsedTagGroups).mockRejectedValue(new Error("保存できない"));
  const { user } = await renderBar();

  await user.click(groups()[0]);
  await settle();

  expect(groups()[0]).toHaveAttribute("aria-expanded", "false");
  expect(console.error).toHaveBeenCalled();
});

test("閉じたグループの中に選択中のタグがあれば、選択数のバッジを出す", async () => {
  vi.mocked(api.getCollapsedTagGroups).mockResolvedValue([10]);
  await renderBar([2, 3, 1]);

  expect(screen.getByTestId("tag-bar-group-badge")).toHaveTextContent("2");
  // 猫はグループ外なのでバッジに数えない。
  expect(screen.getAllByTestId("tag-bar-group-badge")).toHaveLength(1);
});

test("バッジは、開いているグループ・選択のないグループには出さない", async () => {
  vi.mocked(api.getCollapsedTagGroups).mockResolvedValue([10]);
  await renderBar([1]);
  expect(screen.queryByTestId("tag-bar-group-badge")).toBeNull();
});

test("開いているグループでは、選択があってもバッジを出さない", async () => {
  await renderBar([2]);
  expect(screen.queryByTestId("tag-bar-group-badge")).toBeNull();
});

test("グループ内のタグのクリックは、グループなしのタグと同じく onChange に反映する", async () => {
  const { user, onChange } = await renderBar([1]);

  await user.click(chips()[1]); // 犬
  expect(onChange).toHaveBeenLastCalledWith([1, 2]);
});

test("選択済みのグループ内のタグをクリックすると、選択から外す", async () => {
  const { user, onChange } = await renderBar([1, 2]);

  await user.click(chips()[1]);
  expect(onChange).toHaveBeenLastCalledWith([1]);
});

test("グループ行のクリックは、選択を変えない", async () => {
  const { user, onChange } = await renderBar([2]);

  await user.click(groups()[0]);
  await user.click(groups()[1]);
  expect(onChange).not.toHaveBeenCalled();
});

test("すべて解除は、グループ内のものも含め選択を空にする", async () => {
  const { user, onChange } = await renderBar([1, 2]);

  await user.click(screen.getByTestId("tag-bar-clear-all"));
  expect(onChange).toHaveBeenCalledWith([]);
});

test("タグが0件のときは何も描画しない", async () => {
  vi.mocked(api.getTagLayout).mockResolvedValue([]);
  await renderBar();

  expect(screen.queryByTestId("tag-bar")).toBeNull();
});

test("読み込み前に開閉しても、あとから届く保存済みの状態で上書きしない", async () => {
  let resolveCollapsed: (ids: number[]) => void = () => {};
  vi.mocked(api.getCollapsedTagGroups).mockReturnValue(
    new Promise<number[]>((resolve) => {
      resolveCollapsed = resolve;
    }),
  );
  const { user } = await renderBar();

  await user.click(groups()[0]);
  expect(groups()[0]).toHaveAttribute("aria-expanded", "false");

  resolveCollapsed([]);
  await settle();

  expect(groups()[0]).toHaveAttribute("aria-expanded", "false");
  expect(api.setCollapsedTagGroups).toHaveBeenLastCalledWith([10]);
});
