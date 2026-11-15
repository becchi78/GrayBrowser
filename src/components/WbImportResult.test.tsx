// .wb インポートの結果画面(WbImportDialog 内の WbImportResult)の現状の動作を固定する特性テスト。
// WbImportResult は export されていないため、完了イベントを流して結果画面を出す。
// 文言の組み立ては lib 側のテストが持つので、ここでは data-testid・ボタン・出し分けだけを見る。
import { act, screen } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import * as api from "../api";
import * as events from "../events";
import type { UnmatchedThumbnailDto, WbImportSummary } from "../types";
import { never } from "../testing/async";
import { makeSummary } from "../testing/fixtures";
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

const EMPTY_SUMMARY = makeSummary();

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
  await screen.findByTestId("import-result-unmatched");

  expect(screen.getByTestId("import-result-unmatched-empty-hash")).toBeInTheDocument();
  expect(screen.getByTestId("import-result-unmatched")).toBeInTheDocument();
  expect(screen.getByTestId("import-result-invalid-name")).toBeInTheDocument();
  expect(screen.getByTestId("import-result-failed-rows")).toBeInTheDocument();
  expect(screen.getByTestId("import-result-skipped-roots")).toBeInTheDocument();
  expect(screen.getByTestId("import-result-volume-label-unresolved")).toBeInTheDocument();
});

test("「確認する」は自動対応できなかった項目にだけ出る", async () => {
  renderDialog();
  completeImport(ALL_ATTENTION_SUMMARY);
  await screen.findByTestId("import-result-unmatched");

  expect(screen.getByTestId("import-unmatched-review-btn-unmatched")).toBeInTheDocument();
  expect(screen.getByTestId("import-unmatched-review-btn-unmatched-empty-hash")).toBeInTheDocument();
  expect(screen.queryByTestId("import-unmatched-review-btn-failed")).toBeNull();
  expect(screen.queryByTestId("import-unmatched-review-btn-invalid-name")).toBeNull();
  expect(screen.queryByTestId("import-unmatched-review-btn-failed-rows")).toBeNull();
});

test.each([
  { cause: "対応する動画がない", button: "unmatched", rowCount: 2 },
  { cause: "ハッシュが空の", button: "unmatched-empty-hash", rowCount: 1 },
])("$cause項目の「確認する」は、その原因の行だけを開く", async ({ button, rowCount }) => {
  const { user } = renderDialog();
  completeImport(ALL_ATTENTION_SUMMARY);

  await user.click(await screen.findByTestId(`import-unmatched-review-btn-${button}`));

  await screen.findByTestId("unmatched-list");
  expect(screen.getAllByTestId("unmatched-row")).toHaveLength(rowCount);
});

test("一覧の読み込み中は、自動対応できなかった項目を出さず、他の項目は出す", async () => {
  vi.mocked(api.listUnmatchedThumbnails).mockReturnValue(never());
  renderDialog();
  completeImport(ALL_ATTENTION_SUMMARY);

  await screen.findByTestId("import-result-failed-rows");

  expect(screen.queryByTestId("import-result-unmatched")).toBeNull();
  expect(screen.queryByTestId("import-result-unmatched-empty-hash")).toBeNull();
});

test("一覧に該当する原因の行が残っていなければ、その項目を出さない", async () => {
  vi.mocked(api.listUnmatchedThumbnails).mockResolvedValue(
    ROWS.filter((r) => r.kind === "empty_hash"),
  );
  renderDialog();
  completeImport(ALL_ATTENTION_SUMMARY);

  await screen.findByTestId("import-result-unmatched-empty-hash");

  expect(screen.queryByTestId("import-result-unmatched")).toBeNull();
});

test("一覧の取得に失敗したときは、自動対応できなかった項目を隠さずに出す", async () => {
  vi.spyOn(console, "error").mockImplementation(() => {});
  vi.mocked(api.listUnmatchedThumbnails).mockRejectedValue("失敗");
  renderDialog();
  completeImport(ALL_ATTENTION_SUMMARY);

  await screen.findByTestId("import-result-unmatched");

  expect(screen.getByTestId("import-result-unmatched-empty-hash")).toBeInTheDocument();
});
