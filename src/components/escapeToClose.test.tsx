// 各ダイアログが Escape で閉じる条件の現状を固定する特性テスト。
// 「閉じない」ダイアログのテストは、仕様を変えるときに意図的に更新する前提の記録。
import { beforeEach, expect, test, vi } from "vitest";
import { screen, waitFor } from "@testing-library/react";
import { getVersion } from "@tauri-apps/api/app";
import * as api from "../api";
import { renderWithProviders } from "../testing/render";
import { AboutDialog } from "./AboutDialog";
import { BulkTagDialog } from "./BulkTagDialog";
import { ConfirmDialog } from "./ConfirmDialog";
import { FolderDialog } from "./FolderDialog";
import { LogViewerDialog } from "./LogViewerDialog";
import { SettingsDialog } from "./SettingsDialog";
import { TagBarEditDialog } from "./TagBarEditDialog";
import { UnmatchedThumbnailsDialog } from "./UnmatchedThumbnailsDialog";
import { UpdateDialog } from "./UpdateDialog";
import { WbImportDialog } from "./WbImportDialog";

// 各ダイアログが開いた直後に読む関数だけをフェイクにする。api.ts 全体を
// 差し替えるのは、未モックの関数が実 IPC に触れて例外になるのを避けるため。
vi.mock("../api", () => ({
  getNasPollIntervalMinutes: vi.fn(),
  getUpdateCheckOnStartup: vi.fn(),
  getRestoreViewStateEnabled: vi.fn(),
  setNasPollIntervalMinutes: vi.fn(),
  setUpdateCheckOnStartup: vi.fn(),
  setRestoreViewStateEnabled: vi.fn(),
  checkForUpdate: vi.fn(),
  listAllTags: vi.fn(),
  assignTagToVideos: vi.fn(),
  readLog: vi.fn(),
  getLogDir: vi.fn(),
  listUnmatchedThumbnails: vi.fn(),
  listWatchFolders: vi.fn(),
  listWatchCheckDisabledFolders: vi.fn(),
  listTagsWithVideoCount: vi.fn(),
}));

// WbImportDialog が購読するイベントは、購読解除関数を返すだけの何もしないフェイクにする。
vi.mock("../events", () => ({
  onWbImportProgress: vi.fn(() => () => {}),
  onWbImportStage: vi.fn(() => () => {}),
  onWbImportComplete: vi.fn(() => () => {}),
  onWbImportFailed: vi.fn(() => () => {}),
}));

/** 解決しない Promise。「処理中」の状態を保つために使う。 */
function never<T>(): Promise<T> {
  return new Promise<T>(() => {});
}

beforeEach(() => {
  vi.mocked(getVersion).mockResolvedValue("1.2.3");
  vi.mocked(api.getNasPollIntervalMinutes).mockResolvedValue(5);
  vi.mocked(api.getUpdateCheckOnStartup).mockResolvedValue(true);
  vi.mocked(api.getRestoreViewStateEnabled).mockResolvedValue(true);
  vi.mocked(api.listAllTags).mockResolvedValue([]);
  vi.mocked(api.readLog).mockResolvedValue({ entries: [], next_offset: 0, reset: false });
  vi.mocked(api.getLogDir).mockResolvedValue("C:\\logs");
  vi.mocked(api.listUnmatchedThumbnails).mockResolvedValue([]);
  vi.mocked(api.listWatchFolders).mockResolvedValue([]);
  vi.mocked(api.listWatchCheckDisabledFolders).mockResolvedValue([]);
  vi.mocked(api.listTagsWithVideoCount).mockResolvedValue([]);
});

// ---- ConfirmDialog ----

function renderConfirm(props: { open: boolean; busy?: boolean }) {
  const onCancel = vi.fn();
  const view = renderWithProviders(
    <ConfirmDialog
      open={props.open}
      busy={props.busy}
      title="確認"
      confirmLabel="実行"
      onConfirm={() => {}}
      onCancel={onCancel}
    >
      本文
    </ConfirmDialog>,
  );
  return { ...view, onCancel };
}

test("ConfirmDialog は open かつ busy でないとき Escape で onCancel を呼ぶ", async () => {
  const { user, onCancel } = renderConfirm({ open: true });

  await user.keyboard("{Escape}");

  expect(onCancel).toHaveBeenCalledTimes(1);
});

test("ConfirmDialog は busy の間は Escape で閉じない", async () => {
  const { user, onCancel } = renderConfirm({ open: true, busy: true });

  await user.keyboard("{Escape}");

  expect(onCancel).not.toHaveBeenCalled();
});

test("ConfirmDialog は open が false なら描画されず、Escape でも onCancel を呼ばない", async () => {
  const { user, onCancel } = renderConfirm({ open: false });

  await user.keyboard("{Escape}");

  expect(screen.queryByTestId("confirm-dialog")).toBeNull();
  expect(onCancel).not.toHaveBeenCalled();
});

// ---- SettingsDialog ----

test("SettingsDialog は保存中でなければ Escape で onClose を呼ぶ", async () => {
  const onClose = vi.fn();
  const { user } = renderWithProviders(<SettingsDialog open={true} onClose={onClose} />);
  await waitFor(() => expect(screen.getByTestId("settings-nas-interval-input")).toBeEnabled());

  await user.keyboard("{Escape}");

  expect(onClose).toHaveBeenCalledTimes(1);
});

test("SettingsDialog は保存中は Escape で閉じない", async () => {
  vi.mocked(api.setNasPollIntervalMinutes).mockReturnValue(never());
  const onClose = vi.fn();
  const { user } = renderWithProviders(<SettingsDialog open={true} onClose={onClose} />);
  await waitFor(() => expect(screen.getByTestId("settings-dialog-save-btn")).toBeEnabled());
  await user.click(screen.getByTestId("settings-dialog-save-btn"));
  await waitFor(() => expect(screen.getByTestId("settings-dialog-save-btn")).toBeDisabled());

  await user.keyboard("{Escape}");

  expect(onClose).not.toHaveBeenCalled();
});

test("SettingsDialog は設定の読み込み中でも Escape で閉じる", async () => {
  vi.mocked(api.getNasPollIntervalMinutes).mockReturnValue(never());
  const onClose = vi.fn();
  const { user } = renderWithProviders(<SettingsDialog open={true} onClose={onClose} />);
  expect(screen.getByTestId("settings-dialog-save-btn")).toBeDisabled();

  await user.keyboard("{Escape}");

  expect(onClose).toHaveBeenCalledTimes(1);
});

test("SettingsDialog は open が false なら描画されず、Escape でも onClose を呼ばない", async () => {
  const onClose = vi.fn();
  const { user } = renderWithProviders(<SettingsDialog open={false} onClose={onClose} />);

  await user.keyboard("{Escape}");

  expect(screen.queryByTestId("settings-dialog")).toBeNull();
  expect(onClose).not.toHaveBeenCalled();
});

// ---- UpdateDialog ----

function fakeUpdate(downloadAndInstall: () => Promise<void>) {
  return {
    version: "2.0.0",
    currentVersion: "1.0.0",
    body: "ノート",
    close: vi.fn().mockResolvedValue(undefined),
    downloadAndInstall: vi.fn(downloadAndInstall),
  } as unknown as NonNullable<Awaited<ReturnType<typeof api.checkForUpdate>>>;
}

test("UpdateDialog は確認中でも Escape で onClose を呼ぶ", async () => {
  vi.mocked(api.checkForUpdate).mockReturnValue(never());
  const onClose = vi.fn();
  const { user } = renderWithProviders(<UpdateDialog open={true} onClose={onClose} />);
  expect(screen.getByTestId("update-dialog-checking")).toBeInTheDocument();

  await user.keyboard("{Escape}");

  expect(onClose).toHaveBeenCalledTimes(1);
});

test("UpdateDialog は更新の案内が出ている状態で Escape で onClose を呼ぶ", async () => {
  vi.mocked(api.checkForUpdate).mockResolvedValue(fakeUpdate(() => never()));
  const onClose = vi.fn();
  const { user } = renderWithProviders(<UpdateDialog open={true} onClose={onClose} />);
  await screen.findByTestId("update-dialog-available");

  await user.keyboard("{Escape}");

  expect(onClose).toHaveBeenCalledTimes(1);
});

test("UpdateDialog は確認に失敗した状態でも Escape で onClose を呼ぶ", async () => {
  vi.spyOn(console, "error").mockImplementation(() => {});
  vi.mocked(api.checkForUpdate).mockRejectedValue("失敗");
  const onClose = vi.fn();
  const { user } = renderWithProviders(<UpdateDialog open={true} onClose={onClose} />);
  await screen.findByTestId("update-dialog-error");

  await user.keyboard("{Escape}");

  expect(onClose).toHaveBeenCalledTimes(1);
});

test("UpdateDialog はダウンロード中は Escape で閉じない", async () => {
  vi.mocked(api.checkForUpdate).mockResolvedValue(fakeUpdate(() => never()));
  const onClose = vi.fn();
  const { user } = renderWithProviders(<UpdateDialog open={true} onClose={onClose} />);
  await user.click(await screen.findByTestId("update-dialog-apply-btn"));
  await screen.findByTestId("update-dialog-progress");

  await user.keyboard("{Escape}");

  expect(onClose).not.toHaveBeenCalled();
});

test("UpdateDialog は open が false なら描画されず、Escape でも onClose を呼ばない", async () => {
  const onClose = vi.fn();
  const { user } = renderWithProviders(<UpdateDialog open={false} onClose={onClose} />);

  await user.keyboard("{Escape}");

  expect(screen.queryByTestId("update-dialog")).toBeNull();
  expect(onClose).not.toHaveBeenCalled();
});

// ---- BulkTagDialog ----

test("BulkTagDialog は処理中でなければ Escape で onClose を呼ぶ", async () => {
  const onClose = vi.fn();
  const { user } = renderWithProviders(
    <BulkTagDialog videoIds={["a", "b"]} onTagged={() => {}} onClose={onClose} />,
  );
  await screen.findByTestId("bulk-tag-dialog");

  await user.keyboard("{Escape}");

  expect(onClose).toHaveBeenCalledTimes(1);
});

test("BulkTagDialog は付与の処理中は Escape で閉じない", async () => {
  vi.mocked(api.assignTagToVideos).mockReturnValue(never());
  const onClose = vi.fn();
  const { user } = renderWithProviders(
    <BulkTagDialog videoIds={["a"]} onTagged={() => {}} onClose={onClose} />,
  );
  await user.type(await screen.findByTestId("bulk-tag-input"), "タグ");
  await user.click(screen.getByTestId("bulk-tag-submit-btn"));
  await waitFor(() => expect(screen.getByTestId("bulk-tag-cancel-btn")).toBeDisabled());

  await user.keyboard("{Escape}");

  expect(onClose).not.toHaveBeenCalled();
});

test("BulkTagDialog は videoIds が null なら描画されず、Escape でも onClose を呼ばない", async () => {
  const onClose = vi.fn();
  const { user } = renderWithProviders(
    <BulkTagDialog videoIds={null} onTagged={() => {}} onClose={onClose} />,
  );

  await user.keyboard("{Escape}");

  expect(screen.queryByTestId("bulk-tag-dialog")).toBeNull();
  expect(onClose).not.toHaveBeenCalled();
});

// ---- 常に Escape で閉じるダイアログ ----

test("AboutDialog は Escape で onClose を呼ぶ", async () => {
  const onClose = vi.fn();
  const { user } = renderWithProviders(<AboutDialog open={true} onClose={onClose} />);
  await screen.findByText("バージョン 1.2.3");

  await user.keyboard("{Escape}");

  expect(onClose).toHaveBeenCalledTimes(1);
});

test("AboutDialog は open が false なら描画されず、Escape でも onClose を呼ばない", async () => {
  const onClose = vi.fn();
  const { user } = renderWithProviders(<AboutDialog open={false} onClose={onClose} />);

  await user.keyboard("{Escape}");

  expect(screen.queryByTestId("about-dialog")).toBeNull();
  expect(onClose).not.toHaveBeenCalled();
});

test("LogViewerDialog は Escape で onClose を呼ぶ", async () => {
  const onClose = vi.fn();
  const { user } = renderWithProviders(<LogViewerDialog open={true} onClose={onClose} />);
  await screen.findByTestId("log-viewer-dialog");
  await waitFor(() => expect(api.readLog).toHaveBeenCalled());

  await user.keyboard("{Escape}");

  expect(onClose).toHaveBeenCalledTimes(1);
});

test("LogViewerDialog は open が false なら描画されず、Escape でも onClose を呼ばない", async () => {
  const onClose = vi.fn();
  const { user } = renderWithProviders(<LogViewerDialog open={false} onClose={onClose} />);

  await user.keyboard("{Escape}");

  expect(screen.queryByTestId("log-viewer-dialog")).toBeNull();
  expect(onClose).not.toHaveBeenCalled();
});

test("UnmatchedThumbnailsDialog は Escape で onClose を呼ぶ", async () => {
  const onClose = vi.fn();
  const { user } = renderWithProviders(
    <UnmatchedThumbnailsDialog onClose={onClose} onThumbnailsReplaced={() => {}} />,
  );
  await screen.findByTestId("unmatched-empty");

  await user.keyboard("{Escape}");

  expect(onClose).toHaveBeenCalledTimes(1);
});

// ---- 現状は Escape で閉じないダイアログ ----
// 現状は閉じない。仕様を変えるときはこのテストも更新する。

test("FolderDialog は現状 Escape を押しても onClose を呼ばない", async () => {
  const onClose = vi.fn();
  const { user } = renderWithProviders(
    <FolderDialog open={true} onClose={onClose} onChanged={() => {}} />,
  );
  await screen.findByTestId("folder-dialog");
  await waitFor(() => expect(api.listWatchCheckDisabledFolders).toHaveBeenCalled());

  await user.keyboard("{Escape}");

  expect(onClose).not.toHaveBeenCalled();
  expect(screen.getByTestId("folder-dialog")).toBeInTheDocument();
});

test("TagBarEditDialog は現状 Escape を押しても onClose を呼ばない", async () => {
  const onClose = vi.fn();
  const { user } = renderWithProviders(
    <TagBarEditDialog open={true} onClose={onClose} onChanged={() => {}} onTagDeleted={() => {}} />,
  );
  await screen.findByTestId("tag-bar-edit-dialog");
  await waitFor(() => expect(api.listTagsWithVideoCount).toHaveBeenCalled());

  await user.keyboard("{Escape}");

  expect(onClose).not.toHaveBeenCalled();
  expect(screen.getByTestId("tag-bar-edit-dialog")).toBeInTheDocument();
});

test("WbImportDialog は現状 Escape を押しても onClose を呼ばない", async () => {
  const onClose = vi.fn();
  const { user } = renderWithProviders(
    <WbImportDialog onClose={onClose} onImportComplete={() => {}} onThumbnailsReplaced={() => {}} />,
  );
  await screen.findByTestId("import-dialog");
  await waitFor(() => expect(api.listUnmatchedThumbnails).toHaveBeenCalled());

  await user.keyboard("{Escape}");

  expect(onClose).not.toHaveBeenCalled();
  expect(screen.getByTestId("import-dialog")).toBeInTheDocument();
});
