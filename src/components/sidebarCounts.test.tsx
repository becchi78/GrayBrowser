// 左ペインの件数ピル: counts が渡されたときだけ DOM に出る(どのテーマでも表示する)。
import { screen, within } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import * as api from "../api";
import { settle } from "../testing/async";
import { renderWithProviders } from "../testing/render";
import type { LibraryCounts } from "../types";
import { FolderSidebar } from "./FolderSidebar";
import { RatingBar } from "./RatingBar";
import { TagBar } from "./TagBar";

vi.mock("../api", () => ({
  listWatchFolders: vi.fn(),
  checkFolderReachability: vi.fn(),
  getTagLayout: vi.fn(),
  getCollapsedTagGroups: vi.fn(),
  setCollapsedTagGroups: vi.fn(),
}));
vi.mock("../events", () => ({ onCatalogChanged: vi.fn(() => () => {}) }));

const COUNTS: LibraryCounts = {
  total: 3580,
  folders: [{ folder_path: "D:\\TV", count: 42 }],
  ratings: [{ rating: 3, count: 7 }],
};

beforeEach(() => {
  vi.mocked(api.listWatchFolders).mockResolvedValue(["D:\\TV", "E:\\Empty"]);
  vi.mocked(api.checkFolderReachability).mockResolvedValue(undefined);
  vi.mocked(api.getTagLayout).mockResolvedValue([{ kind: "tag", id: 1, name: "猫", video_count: 5 }]);
  vi.mocked(api.getCollapsedTagGroups).mockResolvedValue([]);
});

const pill = (el: HTMLElement) => within(el).queryByTestId("count-pill");

test("フォルダ: すべて=総数、各フォルダ=その件数。数えられていないフォルダは出さない", async () => {
  renderWithProviders(<FolderSidebar selected={null} onSelect={() => {}} counts={COUNTS} />);
  await settle();

  expect(pill(screen.getByTestId("folder-sidebar-all"))).toHaveTextContent("3,580");
  const [tv, empty] = screen.getAllByTestId("folder-sidebar-item");
  expect(pill(tv)).toHaveTextContent("42");
  expect(pill(empty)).toBeNull();
});

test("フォルダ: counts が無ければピルを出さず、項目の文字も変わらない", async () => {
  renderWithProviders(<FolderSidebar selected={null} onSelect={() => {}} counts={null} />);
  await settle();

  expect(screen.queryAllByTestId("count-pill")).toHaveLength(0);
  expect(screen.getByTestId("folder-sidebar-all")).toHaveTextContent(/^すべて$/);
});

test("レーティング: ALL=総数、★N=その件数(該当なしは 0)", async () => {
  renderWithProviders(<RatingBar value={[]} onChange={() => {}} counts={COUNTS} />);

  const items = screen.getAllByTestId("rating-bar-item");
  expect(pill(items[0])).toHaveTextContent("3,580");
  expect(pill(items[1])).toHaveTextContent("0"); // ★1
  expect(pill(items[3])).toHaveTextContent("7"); // ★3
});

test("レーティング: counts が無ければピルを出さない", () => {
  renderWithProviders(<RatingBar value={[]} onChange={() => {}} />);
  expect(screen.queryAllByTestId("count-pill")).toHaveLength(0);
});

test("タグ: showCounts のときだけ、タグごとの件数を出す", async () => {
  const { rerender } = renderWithProviders(
    <TagBar selected={[]} onChange={() => {}} showCounts />,
  );
  await settle();
  expect(pill(screen.getByTestId("tag-bar-chip"))).toHaveTextContent("5");

  rerender(<TagBar selected={[]} onChange={() => {}} />);
  await settle();
  expect(pill(screen.getByTestId("tag-bar-chip"))).toBeNull();
});
