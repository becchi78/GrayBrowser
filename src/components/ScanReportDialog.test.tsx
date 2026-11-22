// ScanReportDialog が、直近のスキャンの報告を出し、まだ無いときはそう伝え、コピーできることを確かめる。
import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import * as api from "../api";
import { deferred, settle } from "../testing/async";
import { ScanReportDialog } from "./ScanReportDialog";

vi.mock("../api", () => ({
  getScanReport: vi.fn(),
}));

const REPORT = "GrayBrowser scan report\nStage timings:\n  Enumeration: 250 ms (25.0%)";

let writeText: ReturnType<typeof vi.fn>;

beforeEach(() => {
  vi.spyOn(console, "error").mockImplementation(() => {});
  vi.mocked(api.getScanReport).mockResolvedValue(REPORT);
  writeText = vi.fn().mockResolvedValue(undefined);
  Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
});

async function renderDialog() {
  const onClose = vi.fn();
  render(<ScanReportDialog open={true} onClose={onClose} />);
  await settle();
  return { onClose };
}

const text = () => screen.getByTestId("scan-report-text") as HTMLTextAreaElement;
const copyButton = () => screen.getByTestId("scan-report-copy-btn");

test("閉じているときは何も出さず、取得もしない", () => {
  render(<ScanReportDialog open={false} onClose={vi.fn()} />);

  expect(screen.queryByTestId("scan-report-dialog")).toBeNull();
  expect(api.getScanReport).not.toHaveBeenCalled();
});

test("直近のスキャンの報告を表示し、貼り付ける前に確認するよう案内する", async () => {
  await renderDialog();

  expect(text().value).toBe(REPORT);
  expect(screen.getByTestId("scan-report-dialog")).toHaveTextContent("貼り付ける前に内容を確認");
});

test("スキャンがまだ無いときは、その旨を出し、コピーできない", async () => {
  vi.mocked(api.getScanReport).mockResolvedValue(null);
  await renderDialog();

  expect(screen.getByTestId("scan-report-empty")).toHaveTextContent("まだスキャンが実行されていません");
  expect(screen.queryByTestId("scan-report-text")).toBeNull();
  expect(copyButton()).toBeDisabled();
});

test("取得中は、コピーを押せない", async () => {
  const pending = deferred<string | null>();
  vi.mocked(api.getScanReport).mockReturnValue(pending.promise);
  await renderDialog();

  expect(copyButton()).toBeDisabled();
  expect(screen.queryByTestId("scan-report-empty")).toBeNull();
});

test("「コピー」で報告をクリップボードへ書き、結果を知らせる", async () => {
  await renderDialog();

  fireEvent.click(copyButton());
  await settle();

  expect(writeText).toHaveBeenCalledWith(REPORT);
  expect(screen.getByTestId("scan-report-status")).toHaveTextContent("コピーしました");
});

test("コピーに失敗したら、失敗を知らせる", async () => {
  writeText.mockRejectedValue(new Error("denied"));
  await renderDialog();

  fireEvent.click(copyButton());
  await settle();

  expect(screen.getByTestId("scan-report-status")).toHaveTextContent("コピーできませんでした");
});

test("取得に失敗したらエラーを出し、コピーできない", async () => {
  vi.mocked(api.getScanReport).mockRejectedValue(
    "開発者モードがオフのため、実行できません。",
  );
  await renderDialog();

  expect(screen.getByTestId("scan-report-error")).toHaveTextContent("開発者モードがオフ");
  expect(copyButton()).toBeDisabled();
});

test("「閉じる」で閉じる", async () => {
  const { onClose } = await renderDialog();

  fireEvent.click(screen.getByTestId("scan-report-close-btn"));

  expect(onClose).toHaveBeenCalled();
});
