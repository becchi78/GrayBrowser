// DiagnosticsDialog が、取得した診断情報を出し、コピーできることを確かめる。
import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import * as api from "../api";
import { deferred, settle } from "../testing/async";
import { DiagnosticsDialog } from "./DiagnosticsDialog";

vi.mock("../api", () => ({
  getDiagnostics: vi.fn(),
}));

const REPORT = "GrayBrowser diagnostics\nApp version: 0.18.1";

let writeText: ReturnType<typeof vi.fn>;

beforeEach(() => {
  vi.spyOn(console, "error").mockImplementation(() => {});
  vi.mocked(api.getDiagnostics).mockResolvedValue(REPORT);
  writeText = vi.fn().mockResolvedValue(undefined);
  Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
});

async function renderDialog() {
  const onClose = vi.fn();
  render(<DiagnosticsDialog open={true} onClose={onClose} />);
  await settle();
  return { onClose };
}

const text = () => screen.getByTestId("diagnostics-text") as HTMLTextAreaElement;
const copyButton = () => screen.getByTestId("diagnostics-copy-btn");

test("閉じているときは何も出さず、取得もしない", () => {
  render(<DiagnosticsDialog open={false} onClose={vi.fn()} />);

  expect(screen.queryByTestId("diagnostics-dialog")).toBeNull();
  expect(api.getDiagnostics).not.toHaveBeenCalled();
});

test("取得した診断情報を表示し、貼り付ける前に確認するよう案内する", async () => {
  await renderDialog();

  expect(text().value).toBe(REPORT);
  expect(screen.getByTestId("diagnostics-dialog")).toHaveTextContent("貼り付ける前に内容を確認");
});

test("診断情報の「この版の状態」の行を、そのまま表示する", async () => {
  for (const line of ["Release status: preview (not yet published)", "Release status: unknown"]) {
    vi.mocked(api.getDiagnostics).mockResolvedValue(`GrayBrowser diagnostics
App version: 0.18.2
${line}`);
    const { unmount } = render(<DiagnosticsDialog open={true} onClose={vi.fn()} />);
    await settle();

    expect(text().value).toContain(line);
    unmount();
  }
});

test("取得中は、コピーを押せない", async () => {
  const pending = deferred<string>();
  vi.mocked(api.getDiagnostics).mockReturnValue(pending.promise);
  await renderDialog();

  expect(copyButton()).toBeDisabled();
});

test("「コピー」で診断情報をクリップボードへ書き、結果を知らせる", async () => {
  await renderDialog();

  fireEvent.click(copyButton());
  await settle();

  expect(writeText).toHaveBeenCalledWith(REPORT);
  expect(screen.getByTestId("diagnostics-status")).toHaveTextContent("コピーしました");
});

test("コピーに失敗したら、失敗を知らせる", async () => {
  writeText.mockRejectedValue(new Error("denied"));
  await renderDialog();

  fireEvent.click(copyButton());
  await settle();

  expect(screen.getByTestId("diagnostics-status")).toHaveTextContent("コピーできませんでした");
});

test("取得に失敗したらエラーを出し、コピーできない", async () => {
  vi.mocked(api.getDiagnostics).mockRejectedValue(new Error("取得できません"));
  await renderDialog();

  expect(screen.getByTestId("diagnostics-error")).toHaveTextContent("取得できません");
  expect(copyButton()).toBeDisabled();
});

test("「閉じる」で閉じる", async () => {
  const { onClose } = await renderDialog();

  fireEvent.click(screen.getByTestId("diagnostics-close-btn"));

  expect(onClose).toHaveBeenCalled();
});
