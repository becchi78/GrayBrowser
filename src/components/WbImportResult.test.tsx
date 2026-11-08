// .wb インポートの結果画面(WbImportDialog 内の WbImportResult)の現状の動作を固定する特性テスト。
// WbImportResult は export されていないため、完了イベントを流して結果画面を出す。
// 文言の組み立ては lib 側のテストが持つので、ここでは data-testid・ボタン・出し分けだけを見る。
import { act, screen } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import * as api from "../api";
import * as events from "../events";
import type { UnmatchedThumbnailDto, WbImportSummary } from "../types";
import { renderWithProviders } from "../testing/render";
import { WbImportDialog } from "./WbImportDialog";

vi.mock("../api", () => ({
  listUnmatchedThumbnails: vi.fn(),
}));

vi.mock("../events", () => ({
  onWbImportProgress: vi.fn(() => () => {}),
  onWbImportStage: vi.fn(() => () => {}),
  onWbImportComplete: vi.fn(() => () => {}),
  onWbImportFailed: vi.fn(() => () => {}),
}));

const EMPTY_SUMMARY: WbImportSummary = {
  total: 0,
  registered: 0,
  registered_online: 0,
  registered_offline: 0,
  failed_rows: 0,
  skipped: 0,
  skipped_invalid_name: 0,
  clamped_scores: 0,
  tags_assigned: 0,
  source_tag_count: 0,
  ratings_from_star_tags: 0,
  thumbnails_linked: 0,
  thumbnails_linked_by_name: 0,
  thumbnails_failed: 0,
  thumbnails_unmatched: 0,
  thumbnails_unmatched_empty_hash: 0,
  thumbnails_skipped_online: 0,
  watch_folders_added: [],
  watch_folders_skipped_roots: [],
  volume_label_resolved: 0,
  volume_label_unresolved: 0,
  volume_label_unresolved_labels: [],
};

/** すべての注意項目が出る集計。tags は元データにタグがあるのに付与 0 件(amber)。 */
const ALL_ATTENTION_SUMMARY: WbImportSummary = {
  ...EMPTY_SUMMARY,
  total: 6,
  registered: 1,
  registered_online: 1,
  failed_rows: 1,
  skipped_invalid_name: 1,
  source_tag_count: 5,
  thumbnails_failed: 1,
  thumbnails_unmatched: 3,
  thumbnails_unmatched_empty_hash: 1,
  watch_folders_skipped_roots: ["D:\\"],
  volume_label_unresolved: 1,
  volume_label_unresolved_labels: ["NAS"],
};

const ROWS: UnmatchedThumbnailDto[] = [
  { folder_path: "C:\\thumbs", file_name: "a.jpg", kind: "no_matching_video", default_search_term: null },
  { folder_path: "C:\\thumbs", file_name: "b.jpg", kind: "no_matching_video", default_search_term: null },
  { folder_path: "C:\\thumbs", file_name: "c.jpg", kind: "empty_hash", default_search_term: null },
];

function never<T>(): Promise<T> {
  return new Promise<T>(() => {});
}

function renderDialog() {
  return renderWithProviders(
    <WbImportDialog onClose={() => {}} onImportComplete={() => {}} onThumbnailsReplaced={() => {}} />,
  );
}

/** インポート完了イベントを流して結果画面へ進める。 */
function completeImport(summary: WbImportSummary) {
  const subscribe = vi.mocked(events.onWbImportComplete);
  const callback = subscribe.mock.calls[0][0];
  act(() => callback(summary));
}

beforeEach(() => {
  vi.mocked(api.listUnmatchedThumbnails).mockResolvedValue(ROWS);
});

test("注意項目ごとに決まった data-testid が付く", async () => {
  renderDialog();
  completeImport(ALL_ATTENTION_SUMMARY);
  await screen.findByTestId("import-result-needs-review");

  expect(screen.getByTestId("import-result-empty-hash")).toBeInTheDocument();
  expect(screen.getByTestId("import-result-needs-review")).toBeInTheDocument();
  expect(screen.getByTestId("import-result-skipped-invalid-name")).toBeInTheDocument();
  expect(screen.getByTestId("import-result-failed-rows")).toBeInTheDocument();
  expect(screen.getByTestId("import-result-skipped-roots")).toBeInTheDocument();
  expect(screen.getByTestId("import-result-volume-label-unresolved")).toBeInTheDocument();
});

test("旧サムネイル変換の失敗とタグの項目には専用の data-testid が付かない", async () => {
  renderDialog();
  completeImport(ALL_ATTENTION_SUMMARY);
  await screen.findByTestId("import-result-needs-review");

  // 項目自身に testid が無ければ、最も近い testid は外側のダイアログのものになる。
  const failed = screen.getByText(/旧サムネイル変換に失敗/);
  expect(failed.closest("[data-testid]")).toHaveAttribute("data-testid", "import-dialog");
  const tags = screen.getByText(/元データには5件のタグがありましたが/);
  expect(tags.closest("[data-testid]")).toHaveAttribute("data-testid", "import-dialog");
});

test("「確認する」は自動対応できなかった項目にだけ出る", async () => {
  renderDialog();
  completeImport(ALL_ATTENTION_SUMMARY);
  await screen.findByTestId("import-result-needs-review");

  expect(screen.getByTestId("import-unmatched-review-btn-unmatched")).toBeInTheDocument();
  expect(screen.getByTestId("import-unmatched-review-btn-unmatched-empty-hash")).toBeInTheDocument();
  expect(screen.queryByTestId("import-unmatched-review-btn-failed")).toBeNull();
  expect(screen.queryByTestId("import-unmatched-review-btn-invalid-name")).toBeNull();
  expect(screen.queryByTestId("import-unmatched-review-btn-failed-rows")).toBeNull();
});

test("対応する動画がない項目の「確認する」は、その原因の行だけを開く", async () => {
  const { user } = renderDialog();
  completeImport(ALL_ATTENTION_SUMMARY);

  await user.click(await screen.findByTestId("import-unmatched-review-btn-unmatched"));

  await screen.findByTestId("unmatched-list");
  expect(screen.getAllByTestId("unmatched-row")).toHaveLength(2);
});

test("ハッシュが空の項目の「確認する」は、その原因の行だけを開く", async () => {
  const { user } = renderDialog();
  completeImport(ALL_ATTENTION_SUMMARY);

  await user.click(await screen.findByTestId("import-unmatched-review-btn-unmatched-empty-hash"));

  await screen.findByTestId("unmatched-list");
  expect(screen.getAllByTestId("unmatched-row")).toHaveLength(1);
});

test("一覧の読み込み中は、自動対応できなかった項目を出さず、他の項目は出す", async () => {
  vi.mocked(api.listUnmatchedThumbnails).mockReturnValue(never());
  renderDialog();
  completeImport(ALL_ATTENTION_SUMMARY);

  await screen.findByTestId("import-result-failed-rows");

  expect(screen.queryByTestId("import-result-needs-review")).toBeNull();
  expect(screen.queryByTestId("import-result-empty-hash")).toBeNull();
});

test("一覧に該当する原因の行が残っていなければ、その項目を出さない", async () => {
  vi.mocked(api.listUnmatchedThumbnails).mockResolvedValue(
    ROWS.filter((r) => r.kind === "empty_hash"),
  );
  renderDialog();
  completeImport(ALL_ATTENTION_SUMMARY);

  await screen.findByTestId("import-result-empty-hash");

  expect(screen.queryByTestId("import-result-needs-review")).toBeNull();
});

test("一覧の取得に失敗したときは、自動対応できなかった項目を隠さずに出す", async () => {
  vi.spyOn(console, "error").mockImplementation(() => {});
  vi.mocked(api.listUnmatchedThumbnails).mockRejectedValue("失敗");
  renderDialog();
  completeImport(ALL_ATTENTION_SUMMARY);

  await screen.findByTestId("import-result-needs-review");

  expect(screen.getByTestId("import-result-empty-hash")).toBeInTheDocument();
});
