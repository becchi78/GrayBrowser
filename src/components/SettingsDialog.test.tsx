// SettingsDialog の保存と「既定に戻す」の動作を確かめる。
// 閉じ方・フォーカスなどダイアログ共通の規則は dialogs.test が持つので、ここでは設定の内容を見る。
import { screen, waitFor } from "@testing-library/react";
import type { UserEvent } from "@testing-library/user-event";
import { beforeEach, expect, test, vi } from "vitest";
import * as api from "../api";
import { deferred, settle } from "../testing/async";
import { renderWithProviders } from "../testing/render";
import { SettingsDialog } from "./SettingsDialog";

vi.mock("../api", () => ({
  getNasPollIntervalMinutes: vi.fn(),
  getUpdateCheckOnStartup: vi.fn(),
  getRestoreViewStateEnabled: vi.fn(),
  setNasPollIntervalMinutes: vi.fn(),
  setUpdateCheckOnStartup: vi.fn(),
  setRestoreViewStateEnabled: vi.fn(),
  getRememberWindowSizeEnabled: vi.fn(),
  setRememberWindowSizeEnabled: vi.fn(),
}));

beforeEach(() => {
  vi.spyOn(console, "error").mockImplementation(() => {});
  vi.mocked(api.getNasPollIntervalMinutes).mockResolvedValue(5);
  vi.mocked(api.getUpdateCheckOnStartup).mockResolvedValue(false);
  vi.mocked(api.getRestoreViewStateEnabled).mockResolvedValue(false);
  vi.mocked(api.getRememberWindowSizeEnabled).mockResolvedValue(false);
  vi.mocked(api.setNasPollIntervalMinutes).mockResolvedValue(5);
  vi.mocked(api.setUpdateCheckOnStartup).mockResolvedValue(undefined);
  vi.mocked(api.setRestoreViewStateEnabled).mockResolvedValue(undefined);
  vi.mocked(api.setRememberWindowSizeEnabled).mockResolvedValue(undefined);
});

async function renderDialog() {
  const onClose = vi.fn();
  const view = renderWithProviders(<SettingsDialog open={true} onClose={onClose} />);
  await waitFor(() => expect(interval()).toBeEnabled());
  return { ...view, onClose };
}

const interval = () => screen.getByTestId("settings-nas-interval-input");
const updateCheck = () => screen.getByTestId("settings-update-check-input");
const restoreView = () => screen.getByTestId("settings-restore-view-input");
const rememberWindowSize = () => screen.getByTestId("settings-remember-window-size-input");
const save = () => screen.getByTestId("settings-dialog-save-btn");
const errorText = () => screen.queryByTestId("settings-dialog-error");

async function setInterval(user: UserEvent, value: string) {
  await user.clear(interval());
  await user.type(interval(), value);
}

test("開くと、保存されている設定を欄に反映する", async () => {
  await renderDialog();

  expect(interval()).toHaveValue("5");
  expect(updateCheck()).not.toBeChecked();
  expect(restoreView()).not.toBeChecked();
  expect(rememberWindowSize()).not.toBeChecked();
});

test("ウィンドウの大きさの記憶が有効なら、チェックを入れて表示する", async () => {
  vi.mocked(api.getRememberWindowSizeEnabled).mockResolvedValue(true);
  await renderDialog();

  expect(rememberWindowSize()).toBeChecked();
});

test("設定を取得できないと、エラーを出す", async () => {
  vi.mocked(api.getUpdateCheckOnStartup).mockRejectedValue(new Error("読めない"));
  renderWithProviders(<SettingsDialog open={true} onClose={vi.fn()} />);

  expect(await screen.findByTestId("settings-dialog-error")).toHaveTextContent(
    "現在の設定を取得できませんでした: 読めない",
  );
});

test("保存すると、4つの設定を順に書き込み、書き込めたら閉じる", async () => {
  const order: string[] = [];
  vi.mocked(api.setNasPollIntervalMinutes).mockImplementation(async (m) => {
    order.push("nas");
    return m;
  });
  vi.mocked(api.setUpdateCheckOnStartup).mockImplementation(async () => void order.push("update"));
  vi.mocked(api.setRestoreViewStateEnabled).mockImplementation(
    async () => void order.push("restore"),
  );
  vi.mocked(api.setRememberWindowSizeEnabled).mockImplementation(
    async () => void order.push("windowSize"),
  );
  const { user, onClose } = await renderDialog();
  await setInterval(user, "30");
  await user.click(updateCheck());
  await user.click(rememberWindowSize());

  await user.click(save());

  expect(api.setNasPollIntervalMinutes).toHaveBeenCalledWith(30);
  expect(api.setUpdateCheckOnStartup).toHaveBeenCalledWith(true);
  expect(api.setRestoreViewStateEnabled).toHaveBeenCalledWith(false);
  expect(api.setRememberWindowSizeEnabled).toHaveBeenCalledWith(true);
  expect(order).toEqual(["nas", "update", "restore", "windowSize"]);
  expect(onClose).toHaveBeenCalledTimes(1);
});

test("確認間隔が不正なら、理由を出し、何も書き込まず、閉じない", async () => {
  const { user, onClose } = await renderDialog();
  await setInterval(user, "0");

  await user.click(save());

  expect(errorText()).toHaveTextContent("1〜1440分の範囲");
  expect(api.setNasPollIntervalMinutes).not.toHaveBeenCalled();
  expect(onClose).not.toHaveBeenCalled();
});

test("書き込みが途中で失敗すると、後続は書き込まず、エラーを出し、閉じずに保存し直せる", async () => {
  vi.mocked(api.setUpdateCheckOnStartup).mockRejectedValueOnce(new Error("書き込めない"));
  const { user, onClose } = await renderDialog();

  await user.click(save());

  expect(errorText()).toHaveTextContent("書き込めない");
  expect(api.setNasPollIntervalMinutes).toHaveBeenCalledTimes(1);
  expect(api.setRestoreViewStateEnabled).not.toHaveBeenCalled();
  expect(onClose).not.toHaveBeenCalled();
  expect(save()).toBeEnabled();

  await user.click(save());

  expect(onClose).toHaveBeenCalledTimes(1);
  expect(api.setRestoreViewStateEnabled).toHaveBeenCalledTimes(1);
});

test("ウィンドウの大きさの記憶の書き込みが失敗すると、エラーを出し、閉じずに保存し直せる", async () => {
  vi.mocked(api.setRememberWindowSizeEnabled).mockRejectedValueOnce(new Error("大きさを書き込めない"));
  const { user, onClose } = await renderDialog();

  await user.click(save());

  expect(errorText()).toHaveTextContent("大きさを書き込めない");
  expect(onClose).not.toHaveBeenCalled();
  expect(save()).toBeEnabled();

  await user.click(save());

  expect(onClose).toHaveBeenCalledTimes(1);
  expect(api.setRememberWindowSizeEnabled).toHaveBeenCalledTimes(2);
});

test("保存中は欄とボタンを無効にする", async () => {
  const pending = deferred<number>();
  vi.mocked(api.setNasPollIntervalMinutes).mockReturnValue(pending.promise);
  const { user } = await renderDialog();

  await user.click(save());

  expect(save()).toBeDisabled();
  expect(interval()).toBeDisabled();
  expect(screen.getByTestId("settings-dialog-reset-btn")).toBeDisabled();
  expect(screen.getByTestId("settings-dialog-cancel-btn")).toBeDisabled();
});

test("保存の完了前に閉じられていたら、完了しても閉じる操作を重ねない", async () => {
  const pending = deferred<void>();
  vi.mocked(api.setRestoreViewStateEnabled).mockReturnValue(pending.promise);
  const { user, onClose, unmount } = await renderDialog();
  await user.click(save());
  await waitFor(() => expect(api.setRestoreViewStateEnabled).toHaveBeenCalled());

  unmount();
  pending.resolve();
  await settle();

  expect(onClose).not.toHaveBeenCalled();
});

test("「既定に戻す」で、確認間隔 10 分・両方のチェックありに戻し、エラーを消す。書き込みはまだしない", async () => {
  const { user } = await renderDialog();
  await setInterval(user, "abc");
  await user.click(save());
  expect(errorText()).not.toBeNull();

  await user.click(screen.getByTestId("settings-dialog-reset-btn"));

  expect(interval()).toHaveValue("10");
  expect(updateCheck()).toBeChecked();
  expect(restoreView()).toBeChecked();
  expect(rememberWindowSize()).toBeChecked();
  expect(errorText()).toBeNull();
  expect(api.setNasPollIntervalMinutes).not.toHaveBeenCalled();

  await user.click(save());

  expect(api.setNasPollIntervalMinutes).toHaveBeenCalledWith(10);
  expect(api.setUpdateCheckOnStartup).toHaveBeenCalledWith(true);
  expect(api.setRestoreViewStateEnabled).toHaveBeenCalledWith(true);
  expect(api.setRememberWindowSizeEnabled).toHaveBeenCalledWith(true);
});

test("キャンセルすると、何も書き込まずに閉じる", async () => {
  const { user, onClose } = await renderDialog();
  await setInterval(user, "30");

  await user.click(screen.getByTestId("settings-dialog-cancel-btn"));

  expect(onClose).toHaveBeenCalledTimes(1);
  expect(api.setNasPollIntervalMinutes).not.toHaveBeenCalled();
});
