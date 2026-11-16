// UnmatchedThumbnailsDialog の動画の選択(LinkPicker)・紐付け・スキップの動作を確かめる。
// 検索は入力から 300ms 待って発行されるため、フェイクタイマーで時間を進めて決定的に確かめる。
// フェイクタイマーの下では user-event の待ち合わせが進まないので、操作は fireEvent で行う。
import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import * as api from "../api";
import { deferred, settle } from "../testing/async";
import { UNMATCHED_ROW } from "../testing/fixtures";
import type { LinkCandidateDto, UnmatchedThumbnailDto } from "../types";
import { UnmatchedThumbnailsDialog } from "./UnmatchedThumbnailsDialog";

vi.mock("../api", () => ({
  listUnmatchedThumbnails: vi.fn(),
  searchLinkCandidates: vi.fn(),
  linkUnmatchedThumbnail: vi.fn(),
  dismissUnmatchedThumbnail: vi.fn(),
}));

const ROW_B: UnmatchedThumbnailDto = {
  folder_path: "C:\\old",
  file_name: "b.jpg",
  kind: "no_matching_video",
  default_search_term: null,
};

const CAND_PLAIN: LinkCandidateDto = {
  id: "v1",
  file_name: "one.mp4",
  file_path: "D:\\videos\\one.mp4",
  thumbnail_ready: false,
};
const CAND_READY: LinkCandidateDto = {
  id: "v2",
  file_name: "two.mp4",
  file_path: "D:\\videos\\two.mp4",
  thumbnail_ready: true,
};

beforeEach(() => {
  vi.useFakeTimers();
  vi.spyOn(console, "error").mockImplementation(() => {});
  vi.mocked(api.listUnmatchedThumbnails).mockResolvedValue([UNMATCHED_ROW, ROW_B]);
  vi.mocked(api.searchLinkCandidates).mockResolvedValue([CAND_PLAIN, CAND_READY]);
  vi.mocked(api.linkUnmatchedThumbnail).mockResolvedValue(undefined);
  vi.mocked(api.dismissUnmatchedThumbnail).mockResolvedValue(undefined);
});

afterEach(() => {
  vi.useRealTimers();
});

async function renderDialog() {
  const onThumbnailsReplaced = vi.fn();
  render(<UnmatchedThumbnailsDialog onClose={vi.fn()} onThumbnailsReplaced={onThumbnailsReplaced} />);
  await settle();
  return { onThumbnailsReplaced };
}

/** 検索の待ち時間(300ms)を進め、発行された検索の応答まで反映させる。 */
async function advance(ms: number) {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(ms);
  });
}

const rows = () => screen.getAllByTestId("unmatched-row");
const searchInput = () => screen.getByTestId("unmatched-picker-search");

/** 1行目を開く。既定の検索語(a)で検索が発行される前の状態で返す。 */
async function openFirstRow() {
  fireEvent.click(within(rows()[0]).getByTestId("unmatched-pick-btn"));
  await settle();
}

/** 1行目を開き、検索の結果が出るまで進める。 */
async function openFirstRowWithCandidates() {
  await openFirstRow();
  await advance(300);
  await settle();
}

function pickCandidate(name: string) {
  fireEvent.click(
    screen.getAllByTestId("unmatched-candidate").find((el) => el.textContent?.includes(name))!,
  );
}

test("行を開いても 300ms 経つまで検索せず、経つと既定の検索語で上限 50 件を検索する", async () => {
  await renderDialog();

  await openFirstRow();
  await advance(299);
  expect(api.searchLinkCandidates).not.toHaveBeenCalled();

  await advance(1);
  expect(api.searchLinkCandidates).toHaveBeenCalledTimes(1);
  expect(api.searchLinkCandidates).toHaveBeenCalledWith("a", 50);
});

test("入力が続いている間は検索せず、止まってから最後の検索語で1回だけ検索する", async () => {
  await renderDialog();
  await openFirstRowWithCandidates();
  vi.mocked(api.searchLinkCandidates).mockClear();

  fireEvent.change(searchInput(), { target: { value: "ab" } });
  await advance(200);
  fireEvent.change(searchInput(), { target: { value: "abc" } });
  await advance(299);
  expect(api.searchLinkCandidates).not.toHaveBeenCalled();

  await advance(1);
  expect(api.searchLinkCandidates).toHaveBeenCalledTimes(1);
  expect(api.searchLinkCandidates).toHaveBeenCalledWith("abc", 50);
});

test("行を閉じると、待ち時間の途中だった検索は発行されない", async () => {
  await renderDialog();
  await openFirstRow();

  fireEvent.click(within(rows()[0]).getByTestId("unmatched-pick-btn"));
  await advance(500);

  expect(api.searchLinkCandidates).not.toHaveBeenCalled();
});

test("候補が見つかると並び、見つからないと案内を出す", async () => {
  await renderDialog();
  await openFirstRowWithCandidates();
  expect(screen.getAllByTestId("unmatched-candidate")).toHaveLength(2);
  expect(screen.queryByTestId("unmatched-picker-empty")).toBeNull();

  vi.mocked(api.searchLinkCandidates).mockResolvedValue([]);
  fireEvent.change(searchInput(), { target: { value: "zzz" } });
  await advance(300);

  expect(screen.queryByTestId("unmatched-candidates")).toBeNull();
  expect(screen.getByTestId("unmatched-picker-empty")).toBeInTheDocument();
});

test("検索が失敗するとエラーを出し、「見つかりません」の案内は出さない", async () => {
  vi.mocked(api.searchLinkCandidates).mockRejectedValue(new Error("検索できない"));
  await renderDialog();

  await openFirstRowWithCandidates();

  expect(screen.getByRole("alert")).toHaveTextContent("検索できない");
  expect(screen.queryByTestId("unmatched-picker-empty")).toBeNull();
});

test("古い検索の応答が新しい検索の応答より後に届いても、新しい結果を保つ", async () => {
  const first = deferred<LinkCandidateDto[]>();
  vi.mocked(api.searchLinkCandidates).mockReturnValueOnce(first.promise);
  await renderDialog();
  await openFirstRow();
  await advance(300); // 1回目の検索を発行。応答は保留

  vi.mocked(api.searchLinkCandidates).mockResolvedValueOnce([CAND_READY]);
  fireEvent.change(searchInput(), { target: { value: "new" } });
  await advance(300);
  await settle();
  expect(screen.getAllByTestId("unmatched-candidate")).toHaveLength(1);

  await act(async () => first.resolve([CAND_PLAIN]));
  await settle();

  const shown = screen.getAllByTestId("unmatched-candidate");
  expect(shown).toHaveLength(1);
  expect(shown[0]).toHaveTextContent("two.mp4");
});

test("候補を選ぶと確認が出て、サムネイルがある動画には上書きの案内を添える", async () => {
  await renderDialog();
  await openFirstRowWithCandidates();
  expect(screen.queryByTestId("unmatched-confirm")).toBeNull();

  pickCandidate("one.mp4");
  expect(screen.getByTestId("unmatched-confirm")).toHaveTextContent("「one.mp4」");
  expect(screen.queryByTestId("unmatched-overwrite")).toBeNull();

  pickCandidate("two.mp4");
  expect(screen.getByTestId("unmatched-confirm")).toHaveTextContent("「two.mp4」");
  expect(screen.getByTestId("unmatched-overwrite")).toHaveTextContent("置き換えられ");
});

test("選んだ候補が新しい検索結果から消えたら、選択を解除する", async () => {
  await renderDialog();
  await openFirstRowWithCandidates();
  pickCandidate("one.mp4");
  expect(screen.getByTestId("unmatched-confirm")).toBeInTheDocument();

  vi.mocked(api.searchLinkCandidates).mockResolvedValue([CAND_READY]);
  fireEvent.change(searchInput(), { target: { value: "two" } });
  await advance(300);
  await settle();

  expect(screen.queryByTestId("unmatched-confirm")).toBeNull();
});

test("選んだ候補が新しい検索結果にも残っていれば、選択を保つ", async () => {
  await renderDialog();
  await openFirstRowWithCandidates();
  pickCandidate("two.mp4");

  vi.mocked(api.searchLinkCandidates).mockResolvedValue([CAND_READY]);
  fireEvent.change(searchInput(), { target: { value: "two" } });
  await advance(300);
  await settle();

  expect(screen.getByTestId("unmatched-confirm")).toHaveTextContent("「two.mp4」");
});

test("紐付けに成功すると、選んだ動画を渡して実行し、サムネイル更新を通知して行を消す", async () => {
  const { onThumbnailsReplaced } = await renderDialog();
  await openFirstRowWithCandidates();
  pickCandidate("two.mp4");

  fireEvent.click(screen.getByTestId("unmatched-link-btn"));
  await settle();

  expect(api.linkUnmatchedThumbnail).toHaveBeenCalledWith("C:\\old", "a.jpg", "v2");
  expect(onThumbnailsReplaced).toHaveBeenCalledTimes(1);
  expect(onThumbnailsReplaced).toHaveBeenCalledWith("v2");
  expect(rows()).toHaveLength(1);
  expect(screen.queryByTestId("unmatched-picker")).toBeNull();
});

test("紐付けに失敗しても、サムネイル更新を通知し、行を残してエラーを出す", async () => {
  vi.mocked(api.linkUnmatchedThumbnail).mockRejectedValue(new Error("ffmpeg が失敗"));
  const { onThumbnailsReplaced } = await renderDialog();
  await openFirstRowWithCandidates();
  pickCandidate("two.mp4");

  fireEvent.click(screen.getByTestId("unmatched-link-btn"));
  await settle();

  expect(onThumbnailsReplaced).toHaveBeenCalledTimes(1);
  expect(onThumbnailsReplaced).toHaveBeenCalledWith("v2");
  expect(rows()).toHaveLength(2);
  expect(screen.getByTestId("unmatched-row-error")).toHaveTextContent("ffmpeg が失敗");
  expect(screen.getByTestId("unmatched-picker")).toBeInTheDocument();
});

test("紐付け中は操作を無効にして「紐付け中...」を出し、終わると戻る", async () => {
  const link = deferred<void>();
  vi.mocked(api.linkUnmatchedThumbnail).mockReturnValue(link.promise);
  await renderDialog();
  await openFirstRowWithCandidates();
  pickCandidate("one.mp4");

  fireEvent.click(screen.getByTestId("unmatched-link-btn"));
  await settle();

  expect(screen.getByTestId("unmatched-link-btn")).toHaveTextContent("紐付け中...");
  expect(screen.getByTestId("unmatched-link-btn")).toBeDisabled();
  for (const btn of screen.getAllByTestId("unmatched-dismiss-btn")) expect(btn).toBeDisabled();
  for (const btn of screen.getAllByTestId("unmatched-pick-btn")) expect(btn).toBeDisabled();
  expect(searchInput()).toBeDisabled();

  await act(async () => link.resolve());
  await settle();
  for (const btn of screen.getAllByTestId("unmatched-dismiss-btn")) expect(btn).toBeEnabled();
});

test("スキップに成功すると、その行を消し、サムネイル更新は通知しない", async () => {
  const { onThumbnailsReplaced } = await renderDialog();

  fireEvent.click(within(rows()[0]).getByTestId("unmatched-dismiss-btn"));
  await settle();

  expect(api.dismissUnmatchedThumbnail).toHaveBeenCalledWith("C:\\old", "a.jpg");
  expect(rows()).toHaveLength(1);
  expect(rows()[0]).toHaveTextContent("b.jpg");
  expect(onThumbnailsReplaced).not.toHaveBeenCalled();
});

test("スキップに失敗すると、行を残してエラーを出す", async () => {
  vi.mocked(api.dismissUnmatchedThumbnail).mockRejectedValue(new Error("書き込めない"));
  await renderDialog();

  fireEvent.click(within(rows()[0]).getByTestId("unmatched-dismiss-btn"));
  await settle();

  expect(rows()).toHaveLength(2);
  expect(within(rows()[0]).getByTestId("unmatched-row-error")).toHaveTextContent("書き込めない");
});

test("紐付けの開始から再描画までの間に重ねて押されても、紐付けは1回しか走らない", async () => {
  vi.mocked(api.linkUnmatchedThumbnail).mockReturnValue(deferred<void>().promise);
  await renderDialog();
  await openFirstRowWithCandidates();
  pickCandidate("one.mp4");
  const button = screen.getByTestId("unmatched-link-btn");

  // act の中では再描画が保留されるので、ボタンが無効になる前の連打を再現できる。
  act(() => {
    button.click();
    button.click();
  });

  expect(api.linkUnmatchedThumbnail).toHaveBeenCalledTimes(1);
});
