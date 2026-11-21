// 絞り込み中バー: 表示/非表示、チップの個別解除、すべて解除。
import { screen } from "@testing-library/react";
import { act } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import * as api from "../api";
import * as events from "../events";
import { REFETCH_DELAY_MS } from "../useLibraryCounts";
import { settle } from "../testing/async";
import { renderWithProviders } from "../testing/render";
import { FilterBar } from "./FilterBar";

vi.mock("../api", () => ({ listAllTags: vi.fn() }));
vi.mock("../events", () => ({ onCatalogChanged: vi.fn(() => () => {}) }));

beforeEach(() => {
  vi.spyOn(console, "error").mockImplementation(() => {});
  vi.mocked(api.listAllTags).mockResolvedValue([
    { id: 1, name: "猫" },
    { id: 2, name: "犬" },
  ]);
});

function props(over: Partial<Parameters<typeof FilterBar>[0]> = {}) {
  return {
    folderPath: null,
    onFolderPathChange: vi.fn(),
    tagIds: [],
    onTagIdsChange: vi.fn(),
    ratings: [],
    onRatingsChange: vi.fn(),
    tagBarRefreshKey: 0,
    ...over,
  };
}

async function renderBar(over: Partial<Parameters<typeof FilterBar>[0]> = {}) {
  const p = props(over);
  const utils = renderWithProviders(<FilterBar {...p} />);
  await settle();
  return { ...utils, p };
}

const chips = () => screen.queryAllByTestId("filter-bar-chip");

test("何も絞り込んでいなければバーごと描画しない", async () => {
  await renderBar();
  expect(screen.queryByTestId("filter-bar")).toBeNull();
});

test("フォルダ・タグ・★をチップで並べる。件数は出さない(件数は一覧の上のヘッダーが持つ)", async () => {
  await renderBar({ folderPath: "D:\\TV", tagIds: [2, 1], ratings: [3] });

  expect(chips().map((c) => c.getAttribute("data-kind"))).toEqual([
    "folder",
    "tag",
    "tag",
    "rating",
  ]);
  expect(chips()[0]).toHaveTextContent("フォルダD:\\TV");
  expect(chips()[1]).toHaveTextContent("犬");
  expect(chips()[3]).toHaveTextContent("★★★");
  expect(screen.queryByTestId("filter-bar-count")).toBeNull();
  expect(screen.getByTestId("filter-bar")).not.toHaveTextContent("件");
});

test("×は本物の button で、フォルダのチップだけを解除する", async () => {
  const { user, p } = await renderBar({ folderPath: "D:\\TV", tagIds: [1], ratings: [3] });
  const remove = screen.getAllByTestId("filter-bar-chip-remove");
  expect(remove[0].tagName).toBe("BUTTON");

  await user.click(remove[0]);

  expect(p.onFolderPathChange).toHaveBeenCalledWith(null);
  expect(p.onTagIdsChange).not.toHaveBeenCalled();
  expect(p.onRatingsChange).not.toHaveBeenCalled();
});

test("タグの×は、そのタグだけを外した配列を渡す", async () => {
  const { user, p } = await renderBar({ tagIds: [1, 2] });

  await user.click(screen.getByRole("button", { name: "タグ 猫 の絞り込みを解除" }));

  expect(p.onTagIdsChange).toHaveBeenCalledWith([2]);
  expect(p.onFolderPathChange).not.toHaveBeenCalled();
});

test("★の×は、その評価だけを外した配列を渡す", async () => {
  const { user, p } = await renderBar({ ratings: [3, 5] });

  await user.click(screen.getByRole("button", { name: "レーティング ★★★ の絞り込みを解除" }));

  expect(p.onRatingsChange).toHaveBeenCalledWith([5]);
});

test("すべて解除はフォルダ・タグ・★を全部外す", async () => {
  const { user, p } = await renderBar({ folderPath: "D:\\TV", tagIds: [1], ratings: [3] });

  await user.click(screen.getByTestId("filter-bar-clear-all"));

  expect(p.onFolderPathChange).toHaveBeenCalledWith(null);
  expect(p.onTagIdsChange).toHaveBeenCalledWith([]);
  expect(p.onRatingsChange).toHaveBeenCalledWith([]);
});

test("名前の分からないタグだけが選ばれていても、バーとすべて解除は出る", async () => {
  vi.mocked(api.listAllTags).mockResolvedValue([]);
  await renderBar({ tagIds: [99] });

  expect(screen.getByTestId("filter-bar")).toBeInTheDocument();
  expect(chips()).toHaveLength(0);
  expect(screen.getByTestId("filter-bar-clear-all")).toBeInTheDocument();
});

test("既存の左ペインの testid と衝突しない", async () => {
  await renderBar({ tagIds: [1] });
  expect(screen.queryByTestId("tag-bar")).toBeNull();
  expect(screen.queryByTestId("tag-bar-chip")).toBeNull();
  expect(screen.queryByTestId("tag-bar-clear-all")).toBeNull();
});

test("チップ欄は支援技術に変化が伝わるよう aria-live を持つ", async () => {
  await renderBar({ tagIds: [1], ratings: [3] });

  expect(screen.getByRole("list")).toHaveAttribute("aria-live", "polite");
  expect(screen.queryByRole("status")).toBeNull();
});

test("タグで絞り込んでいなければ、タグ名の取得も通知の購読もしない", async () => {
  vi.mocked(api.listAllTags).mockClear();
  vi.mocked(events.onCatalogChanged).mockClear();

  await renderBar({ folderPath: "D:\\TV", ratings: [3] });

  expect(api.listAllTags).not.toHaveBeenCalled();
  expect(events.onCatalogChanged).not.toHaveBeenCalled();
});

test("カタログの変化の通知は、連続しても最後から待って1回だけ取り直す", async () => {
  vi.useFakeTimers();
  try {
    let notify: () => void = () => {};
    vi.mocked(events.onCatalogChanged).mockImplementation((cb) => {
      notify = cb as () => void;
      return () => {};
    });
    vi.mocked(api.listAllTags).mockClear();
    renderWithProviders(<FilterBar {...props({ tagIds: [1] })} />);
    expect(api.listAllTags).toHaveBeenCalledTimes(1);

    act(() => {
      notify();
      notify();
      notify();
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(REFETCH_DELAY_MS - 1);
    });
    expect(api.listAllTags).toHaveBeenCalledTimes(1);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(1);
    });
    expect(api.listAllTags).toHaveBeenCalledTimes(2);
  } finally {
    vi.useRealTimers();
  }
});
