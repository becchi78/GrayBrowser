// 各ダイアログが Escape で閉じる条件を確かめるテスト。どのダイアログも、背景の
// クリックで閉じられるときだけ Escape でも閉じる(条件は ModalShell の dismissible)。
import { useEffect, useState } from "react";
import { beforeEach, expect, test, vi } from "vitest";
import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { getVersion } from "@tauri-apps/api/app";
import * as api from "../api";
import { renderWithProviders } from "../testing/render";
import { AboutDialog } from "./AboutDialog";
import { BulkTagDialog } from "./BulkTagDialog";
import { ConfirmDialog } from "./ConfirmDialog";
import { EditContextMenu } from "./EditContextMenu";
import { FolderDialog } from "./FolderDialog";
import { LogViewerDialog } from "./LogViewerDialog";
import { SettingsDialog } from "./SettingsDialog";
import { TagBarEditDialog } from "./TagBarEditDialog";
import { UnmatchedThumbnailsDialog } from "./UnmatchedThumbnailsDialog";
import { UpdateDialog } from "./UpdateDialog";
import { WbImportDialog } from "./WbImportDialog";

// 各ダイアログが開いた直後や、確かめる操作の途中で呼ぶ関数だけをフェイクにする。
// api.ts 全体を差し替えるのは、未モックの関数が実 IPC に触れて例外になるのを避けるため。
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
  countVideosUnderFolder: vi.fn(),
  removeWatchFolder: vi.fn(),
  listTagsWithVideoCount: vi.fn(),
  setTagOrder: vi.fn(),
  deleteTag: vi.fn(),
  pickWbFile: vi.fn(),
  pickWbThumbnailFolder: vi.fn(),
  startWbImport: vi.fn(),
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

// ---- 子のダイアログを開くダイアログ ----
// 子が開いている間は、親は Escape で閉じない。Escape で閉じるのは子だけで、
// 子が閉じたあとの Escape で親が閉じる。

async function openFolderWithRows() {
  vi.mocked(api.listWatchFolders).mockResolvedValue(["A", "B"]);
  vi.mocked(api.countVideosUnderFolder).mockResolvedValue(3);
  const onClose = vi.fn();
  const view = renderWithProviders(
    <FolderDialog open={true} onClose={onClose} onChanged={() => {}} />,
  );
  await waitFor(() => expect(screen.queryAllByTestId("folder-row")).toHaveLength(2));
  await waitFor(() => expect(api.listWatchCheckDisabledFolders).toHaveBeenCalled());
  await act(async () => {});
  return { ...view, onClose };
}

async function openFolderDeleteConfirm() {
  const opened = await openFolderWithRows();
  const row = screen
    .getAllByTestId("folder-row")
    .find((r) => within(r).queryByTestId("folder-row-path")?.textContent === "B")!;
  await opened.user.click(within(row).getByTestId("folder-row-delete-btn"));
  await screen.findByTestId("confirm-dialog");
  return opened;
}

test("FolderDialog は Escape で onClose を呼ぶ", async () => {
  const { user, onClose } = await openFolderWithRows();

  await user.keyboard("{Escape}");

  expect(onClose).toHaveBeenCalledTimes(1);
});

test("FolderDialog の登録解除の確認は、Escape で確認だけが閉じ、次の Escape で FolderDialog が閉じる", async () => {
  const { user, onClose } = await openFolderDeleteConfirm();

  await user.keyboard("{Escape}");

  expect(screen.queryByTestId("confirm-dialog")).toBeNull();
  expect(onClose).not.toHaveBeenCalled();
  expect(screen.getByTestId("folder-dialog")).toBeInTheDocument();
  expect(api.removeWatchFolder).not.toHaveBeenCalled();

  await user.keyboard("{Escape}");

  expect(onClose).toHaveBeenCalledTimes(1);
});

test("FolderDialog の登録解除の確認は、解除中は Escape で閉じず、FolderDialog も閉じない", async () => {
  vi.mocked(api.removeWatchFolder).mockReturnValue(never());
  const { user, onClose } = await openFolderDeleteConfirm();
  await user.click(screen.getByTestId("confirm-dialog-confirm-btn"));
  await waitFor(() => expect(screen.getByTestId("confirm-dialog-confirm-btn")).toBeDisabled());

  await user.keyboard("{Escape}");

  expect(screen.getByTestId("confirm-dialog")).toBeInTheDocument();
  expect(onClose).not.toHaveBeenCalled();
});

async function openTagBarEdit() {
  vi.mocked(api.listTagsWithVideoCount).mockResolvedValue([{ id: 1, name: "猫", video_count: 2 }]);
  const onClose = vi.fn();
  const view = renderWithProviders(
    <TagBarEditDialog open={true} onClose={onClose} onChanged={() => {}} onTagDeleted={() => {}} />,
  );
  await screen.findByTestId("tag-bar-edit-row");
  return { ...view, onClose };
}

test("TagBarEditDialog は Escape で onClose を呼ぶ", async () => {
  const { user, onClose } = await openTagBarEdit();

  await user.keyboard("{Escape}");

  expect(onClose).toHaveBeenCalledTimes(1);
});

test("TagBarEditDialog は保存中は Escape で閉じない", async () => {
  vi.mocked(api.setTagOrder).mockReturnValue(never());
  const { user, onClose } = await openTagBarEdit();
  await user.click(screen.getByTestId("tag-bar-edit-save-btn"));
  await waitFor(() => expect(screen.getByTestId("tag-bar-edit-save-btn")).toBeDisabled());

  await user.keyboard("{Escape}");

  expect(onClose).not.toHaveBeenCalled();
  expect(screen.getByTestId("tag-bar-edit-dialog")).toBeInTheDocument();
});

test("TagBarEditDialog の削除の確認は、Escape で確認だけが閉じ、次の Escape で TagBarEditDialog が閉じる", async () => {
  const { user, onClose } = await openTagBarEdit();
  await user.click(screen.getByTestId("tag-bar-edit-delete-btn"));
  await screen.findByTestId("confirm-dialog");

  await user.keyboard("{Escape}");

  expect(screen.queryByTestId("confirm-dialog")).toBeNull();
  expect(onClose).not.toHaveBeenCalled();
  expect(screen.getByTestId("tag-bar-edit-dialog")).toBeInTheDocument();
  expect(api.deleteTag).not.toHaveBeenCalled();

  await user.keyboard("{Escape}");

  expect(onClose).toHaveBeenCalledTimes(1);
});

test("TagBarEditDialog は削除中は Escape で、削除の確認も TagBarEditDialog も閉じない", async () => {
  vi.mocked(api.deleteTag).mockReturnValue(never());
  const { user, onClose } = await openTagBarEdit();
  await user.click(screen.getByTestId("tag-bar-edit-delete-btn"));
  await user.click(screen.getByTestId("confirm-dialog-confirm-btn"));
  await waitFor(() => expect(screen.getByTestId("confirm-dialog-confirm-btn")).toBeDisabled());

  await user.keyboard("{Escape}");

  expect(screen.getByTestId("confirm-dialog")).toBeInTheDocument();
  expect(onClose).not.toHaveBeenCalled();
});

async function openWbImport() {
  const onClose = vi.fn();
  const view = renderWithProviders(
    <WbImportDialog onClose={onClose} onImportComplete={() => {}} onThumbnailsReplaced={() => {}} />,
  );
  await screen.findByTestId("import-dialog");
  await waitFor(() => expect(api.listUnmatchedThumbnails).toHaveBeenCalled());
  await act(async () => {});
  return { ...view, onClose };
}

test("WbImportDialog は Escape で onClose を呼ぶ", async () => {
  const { user, onClose } = await openWbImport();

  await user.keyboard("{Escape}");

  expect(onClose).toHaveBeenCalledTimes(1);
});

test("WbImportDialog はインポート中は Escape で閉じない", async () => {
  vi.mocked(api.pickWbFile).mockResolvedValue("C:\\a.wb");
  vi.mocked(api.pickWbThumbnailFolder).mockResolvedValue("C:\\thumbs");
  vi.mocked(api.startWbImport).mockReturnValue(never());
  const { user, onClose } = await openWbImport();
  await user.click(screen.getByRole("button", { name: ".wbファイルを選択" }));
  await user.click(screen.getByRole("button", { name: "旧サムネイルフォルダを選択" }));
  await user.click(screen.getByRole("button", { name: "インポート" }));
  await waitFor(() => expect(screen.getByTestId("import-dialog-close-btn")).toBeDisabled());

  await user.keyboard("{Escape}");

  expect(onClose).not.toHaveBeenCalled();
  expect(screen.getByTestId("import-dialog")).toBeInTheDocument();
});

test("WbImportDialog から開いた UnmatchedThumbnailsDialog は、Escape で子だけが閉じ、次の Escape で WbImportDialog が閉じる", async () => {
  vi.mocked(api.listUnmatchedThumbnails).mockResolvedValue([
    {
      folder_path: "C:\\old",
      file_name: "a.jpg",
      kind: "no_matching_video",
      default_search_term: "a",
    },
  ]);
  const { user, onClose } = await openWbImport();
  await user.click(await screen.findByTestId("import-unmatched-open-btn"));
  await screen.findByTestId("unmatched-row");

  await user.keyboard("{Escape}");

  expect(screen.queryByTestId("unmatched-dialog")).toBeNull();
  expect(onClose).not.toHaveBeenCalled();
  expect(screen.getByTestId("import-dialog")).toBeInTheDocument();

  await user.keyboard("{Escape}");

  expect(onClose).toHaveBeenCalledTimes(1);
});

// ---- 入力欄の編集メニュー ----

/** App と同じく、入力欄の右クリックで編集メニューをダイアログの外に出す。 */
function BulkTagWithEditMenu({ onClose }: { onClose: () => void }) {
  const [target, setTarget] = useState<HTMLInputElement | null>(null);
  useEffect(() => {
    function handler(e: MouseEvent) {
      if (e.target instanceof HTMLInputElement) {
        e.preventDefault();
        setTarget(e.target);
      }
    }
    document.addEventListener("contextmenu", handler);
    return () => document.removeEventListener("contextmenu", handler);
  }, []);
  return (
    <>
      <BulkTagDialog videoIds={["a"]} onTagged={() => {}} onClose={onClose} />
      {target && (
        <EditContextMenu target={target} position={{ x: 0, y: 0 }} onClose={() => setTarget(null)} />
      )}
    </>
  );
}

test("ダイアログの入力欄から開いた編集メニューは、Escape でメニューだけが閉じ、ダイアログは閉じない", async () => {
  const onClose = vi.fn();
  const { user } = renderWithProviders(<BulkTagWithEditMenu onClose={onClose} />);
  const input = await screen.findByTestId("bulk-tag-input");
  fireEvent.contextMenu(input);
  await screen.findByTestId("context-menu");

  await user.keyboard("{Escape}");

  expect(screen.queryByTestId("context-menu")).toBeNull();
  expect(onClose).not.toHaveBeenCalled();
  expect(screen.getByTestId("bulk-tag-dialog")).toBeInTheDocument();

  await user.keyboard("{Escape}");

  expect(onClose).toHaveBeenCalledTimes(1);
});
