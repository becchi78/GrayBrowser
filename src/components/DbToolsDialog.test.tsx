// DbToolsDialog が、サイズと整合性チェックの結果を出し、最適化を確認の後にだけ実行することを確かめる。
import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import * as api from "../api";
import { deferred, settle } from "../testing/async";
import { DbToolsDialog } from "./DbToolsDialog";

vi.mock("../api", () => ({
  getDbStats: vi.fn(),
  runIntegrityCheck: vi.fn(),
  runVacuum: vi.fn(),
}));

const STATS: api.DbStats = {
  file_size: 5_000_000,
  file_size_text: "4.8 MB",
  page_count: 1220,
  freelist_count: 300,
  page_size: 4096,
  reclaimable_text: "1.2 MB",
};

const VACUUMED: api.VacuumResult = {
  size_before: 5_000_000,
  size_after: 3_000_000,
  size_before_text: "4.8 MB",
  size_after_text: "2.9 MB",
  backup_file: "app_manual_20261122.db",
};

beforeEach(() => {
  vi.spyOn(console, "error").mockImplementation(() => {});
  vi.mocked(api.getDbStats).mockResolvedValue(STATS);
  vi.mocked(api.runIntegrityCheck).mockResolvedValue({
    ok: true,
    messages: [],
  });
  vi.mocked(api.runVacuum).mockResolvedValue(VACUUMED);
});

async function renderDialog() {
  const onClose = vi.fn();
  render(<DbToolsDialog open={true} onClose={onClose} />);
  await settle();
  return { onClose };
}

const vacuumButton = () => screen.getByTestId("db-tools-vacuum-btn");

test("閉じているときは何も出さず、取得もしない", () => {
  render(<DbToolsDialog open={false} onClose={vi.fn()} />);

  expect(screen.queryByTestId("db-tools-dialog")).toBeNull();
  expect(api.getDbStats).not.toHaveBeenCalled();
});

test("開くとデータベースのサイズと空きページ数を出す", async () => {
  await renderDialog();

  expect(screen.getByTestId("db-tools-file-size")).toHaveTextContent("4.8 MB");
  expect(screen.getByTestId("db-tools-stats")).toHaveTextContent("1.2 MB");
});

test("サイズの取得に失敗したら、エラーを出す", async () => {
  vi.mocked(api.getDbStats).mockRejectedValue(new Error("読めません"));
  await renderDialog();

  expect(screen.getByTestId("db-tools-stats")).toHaveTextContent("読めません");
});

test("整合性チェックの結果が正常なら、問題なしと出す", async () => {
  await renderDialog();

  fireEvent.click(screen.getByTestId("db-tools-check-btn"));
  await settle();

  expect(screen.getByTestId("db-tools-integrity-ok")).toBeInTheDocument();
});

test("整合性チェックで問題が見つかったら、メッセージを一覧で出す", async () => {
  vi.mocked(api.runIntegrityCheck).mockResolvedValue({
    ok: false,
    messages: ["page 5 is bad", "row 3 missing"],
  });
  await renderDialog();

  fireEvent.click(screen.getByTestId("db-tools-check-btn"));
  await settle();

  const ng = screen.getByTestId("db-tools-integrity-ng");
  expect(ng).toHaveTextContent("page 5 is bad");
  expect(ng).toHaveTextContent("row 3 missing");
  expect(screen.queryByTestId("db-tools-integrity-ok")).toBeNull();
});

test("整合性チェックの実行中は、実行を押せない", async () => {
  const pending = deferred<api.IntegrityResult>();
  vi.mocked(api.runIntegrityCheck).mockReturnValue(pending.promise);
  await renderDialog();

  fireEvent.click(screen.getByTestId("db-tools-check-btn"));
  await settle();

  expect(screen.getByTestId("db-tools-check-btn")).toBeDisabled();
  expect(vacuumButton()).toBeDisabled();
});

test("最適化は確認ダイアログを出し、確認の前には実行しない", async () => {
  await renderDialog();

  fireEvent.click(vacuumButton());
  await settle();

  const confirm = screen.getByTestId("confirm-dialog");
  expect(confirm).toHaveTextContent("バックアップ");
  expect(confirm).toHaveTextContent("元に戻せません");
  expect(api.runVacuum).not.toHaveBeenCalled();
});

test("確認でキャンセルすると、最適化は実行されない", async () => {
  await renderDialog();

  fireEvent.click(vacuumButton());
  await settle();
  fireEvent.click(screen.getByTestId("confirm-dialog-cancel-btn"));
  await settle();

  expect(api.runVacuum).not.toHaveBeenCalled();
  expect(screen.queryByTestId("confirm-dialog")).toBeNull();
});

test("確認すると最適化を実行し、サイズの前後とバックアップ名を出し、サイズを取り直す", async () => {
  await renderDialog();

  fireEvent.click(vacuumButton());
  await settle();
  fireEvent.click(screen.getByTestId("confirm-dialog-confirm-btn"));
  await settle();

  expect(api.runVacuum).toHaveBeenCalledTimes(1);
  const done = screen.getByTestId("db-tools-vacuum-done");
  expect(done).toHaveTextContent("4.8 MB");
  expect(done).toHaveTextContent("2.9 MB");
  expect(done).toHaveTextContent("app_manual_20261122.db");
  expect(screen.queryByTestId("confirm-dialog")).toBeNull();
  expect(api.getDbStats).toHaveBeenCalledTimes(2);
});

test("最適化の実行中は、閉じられず、二重に実行もできない", async () => {
  const pending = deferred<api.VacuumResult>();
  vi.mocked(api.runVacuum).mockReturnValue(pending.promise);
  const { onClose } = await renderDialog();

  fireEvent.click(vacuumButton());
  await settle();
  fireEvent.click(screen.getByTestId("confirm-dialog-confirm-btn"));
  await settle();

  expect(screen.getByTestId("confirm-dialog-confirm-btn")).toBeDisabled();
  expect(screen.getByTestId("db-tools-close-btn")).toBeDisabled();
  fireEvent.keyDown(document, { key: "Escape" });
  expect(onClose).not.toHaveBeenCalled();
  expect(api.runVacuum).toHaveBeenCalledTimes(1);
});

test("最適化の実行中に閉じられても、完了後に状態更新の警告を出さない", async () => {
  const pending = deferred<api.VacuumResult>();
  vi.mocked(api.runVacuum).mockReturnValue(pending.promise);
  const errorSpy = vi.mocked(console.error);
  const { unmount } = render(<DbToolsDialog open={true} onClose={vi.fn()} />);
  await settle();

  fireEvent.click(vacuumButton());
  await settle();
  fireEvent.click(screen.getByTestId("confirm-dialog-confirm-btn"));
  await settle();
  unmount();
  pending.resolve(VACUUMED);
  await settle();

  expect(errorSpy).not.toHaveBeenCalled();
  expect(api.getDbStats).toHaveBeenCalledTimes(1);
});

test("最適化に失敗したら、理由を出す", async () => {
  vi.mocked(api.runVacuum).mockRejectedValue(
    "事前のバックアップに失敗したため、最適化を行いませんでした。",
  );
  await renderDialog();

  fireEvent.click(vacuumButton());
  await settle();
  fireEvent.click(screen.getByTestId("confirm-dialog-confirm-btn"));
  await settle();

  expect(screen.getByTestId("db-tools-vacuum-error")).toHaveTextContent(
    "バックアップに失敗",
  );
  expect(screen.queryByTestId("db-tools-vacuum-done")).toBeNull();
});

test("「閉じる」で閉じる", async () => {
  const { onClose } = await renderDialog();

  fireEvent.click(screen.getByTestId("db-tools-close-btn"));

  expect(onClose).toHaveBeenCalled();
});
