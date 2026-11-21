// 全ダイアログの「閉じ方」と「外側」(背景のクリック・Escape・子のダイアログ・初期フォーカス・
// role/aria 属性・描画の場所)を確かめるテスト。
// どのダイアログも ModalShell を通り、背景のクリックと Escape を同じ条件(dismissible)で
// 扱うので、ダイアログの表(DIALOGS)に対して同じ確認を繰り返す。ModalShell 自身の
// 規則は末尾で単体として確かめる。
import { useEffect, useState, type ReactElement } from "react";
import { beforeEach, expect, test, vi } from "vitest";
import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import type { UserEvent } from "@testing-library/user-event";
import { getVersion } from "@tauri-apps/api/app";
import * as api from "../api";
import { never } from "../testing/async";
import { fakeUpdate } from "../testing/fakes";
import { UNMATCHED_ROW } from "../testing/fixtures";
import { renderWithProviders } from "../testing/render";
import { AboutDialog } from "./AboutDialog";
import { BulkTagDialog } from "./BulkTagDialog";
import { ConfirmDialog } from "./ConfirmDialog";
import { EditContextMenu } from "./EditContextMenu";
import { FolderDialog } from "./FolderDialog";
import { LogViewerDialog } from "./LogViewerDialog";
import { ModalShell } from "./ModalShell";
import { SettingsDialog } from "./SettingsDialog";
import { TagBarEditDialog } from "./TagBarEditDialog";
import { UnmatchedThumbnailsDialog } from "./UnmatchedThumbnailsDialog";
import { UpdateDialog } from "./UpdateDialog";
import { WbImportDialog } from "./WbImportDialog";
import { hasUndismissibleModal } from "../undismissibleModals";

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
  assignTagsToVideos: vi.fn(),
  readLog: vi.fn(),
  getLogDir: vi.fn(),
  listUnmatchedThumbnails: vi.fn(),
  listWatchFolders: vi.fn(),
  listWatchCheckDisabledFolders: vi.fn(),
  countVideosUnderFolder: vi.fn(),
  removeWatchFolder: vi.fn(),
  getTagLayout: vi.fn(),
  moveTagLayoutItem: vi.fn(),
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

beforeEach(() => {
  vi.mocked(getVersion).mockResolvedValue("1.2.3");
  vi.mocked(api.getNasPollIntervalMinutes).mockResolvedValue(5);
  vi.mocked(api.getUpdateCheckOnStartup).mockResolvedValue(true);
  vi.mocked(api.getRestoreViewStateEnabled).mockResolvedValue(true);
  vi.mocked(api.checkForUpdate).mockResolvedValue(fakeUpdate(() => never()));
  vi.mocked(api.listAllTags).mockResolvedValue([]);
  vi.mocked(api.readLog).mockResolvedValue({ entries: [], next_offset: 0, reset: false });
  vi.mocked(api.getLogDir).mockResolvedValue("C:\\logs");
  vi.mocked(api.listUnmatchedThumbnails).mockResolvedValue([]);
  vi.mocked(api.listWatchFolders).mockResolvedValue(["A", "B"]);
  vi.mocked(api.listWatchCheckDisabledFolders).mockResolvedValue([]);
  vi.mocked(api.countVideosUnderFolder).mockResolvedValue(3);
  vi.mocked(api.getTagLayout).mockResolvedValue([
    { kind: "tag", id: 1, name: "猫", video_count: 2 },
  ]);
  vi.mocked(api.pickWbFile).mockResolvedValue("C:\\a.wb");
  vi.mocked(api.pickWbThumbnailFolder).mockResolvedValue("C:\\thumbs");
});

// ---- 開き方 ----

interface Opened {
  user: UserEvent;
  /** `.dialog` の要素。 */
  dialog: HTMLElement;
  /** 閉じる操作(onClose / onCancel)が届いたか。 */
  closed: () => boolean;
}

/** `.dialog` の直接の親が `.dialog-overlay` であることを確かめて返す。 */
function overlayOf(dialog: HTMLElement): HTMLElement {
  const overlay = dialog.parentElement;
  if (overlay === null || !overlay.classList.contains("dialog-overlay")) {
    throw new Error(".dialog の親が .dialog-overlay ではない");
  }
  return overlay;
}

/** 呼ばれたかどうかを見るための onClose / onCancel。 */
function closeSpy() {
  const fn = vi.fn();
  return { fn, closed: () => fn.mock.calls.length > 0 };
}

async function openAbout(): Promise<Opened> {
  const { fn, closed } = closeSpy();
  const { user } = renderWithProviders(<AboutDialog open={true} onClose={fn} />);
  await screen.findByText("バージョン 1.2.3");
  return { user, dialog: screen.getByTestId("about-dialog"), closed };
}

async function openBulkTag(): Promise<Opened> {
  const { fn, closed } = closeSpy();
  const { user } = renderWithProviders(
    <BulkTagDialog videoIds={["a", "b"]} onTagged={() => {}} onClose={fn} />,
  );
  await waitFor(() => expect(api.listAllTags).toHaveBeenCalled());
  await act(async () => {});
  return { user, dialog: screen.getByTestId("bulk-tag-dialog"), closed };
}

function renderConfirm(busy: boolean): Opened {
  const { fn, closed } = closeSpy();
  const { user } = renderWithProviders(
    <ConfirmDialog
      open={true}
      busy={busy}
      title="確認"
      confirmLabel="実行"
      onConfirm={() => {}}
      onCancel={fn}
    >
      本文
    </ConfirmDialog>,
  );
  return { user, dialog: screen.getByTestId("confirm-dialog"), closed };
}

async function openConfirm(): Promise<Opened> {
  return renderConfirm(false);
}

async function openLogViewer(): Promise<Opened> {
  const { fn, closed } = closeSpy();
  const { user } = renderWithProviders(<LogViewerDialog open={true} onClose={fn} />);
  await waitFor(() => expect(api.readLog).toHaveBeenCalled());
  await screen.findByText("ログフォルダ: C:\\logs");
  return { user, dialog: screen.getByTestId("log-viewer-dialog"), closed };
}

async function openSettings(): Promise<Opened> {
  const { fn, closed } = closeSpy();
  const { user } = renderWithProviders(<SettingsDialog open={true} onClose={fn} />);
  await waitFor(() => expect(screen.getByTestId("settings-nas-interval-input")).toBeEnabled());
  return { user, dialog: screen.getByTestId("settings-dialog"), closed };
}

/** 更新の案内(available)が出た状態まで開く。 */
async function openUpdate(): Promise<Opened> {
  const { fn, closed } = closeSpy();
  const { user } = renderWithProviders(<UpdateDialog open={true} onClose={fn} />);
  await screen.findByTestId("update-dialog-available");
  return { user, dialog: screen.getByTestId("update-dialog"), closed };
}

async function openFolder(): Promise<Opened> {
  const { fn, closed } = closeSpy();
  const { user } = renderWithProviders(
    <FolderDialog open={true} onClose={fn} onChanged={() => {}} />,
  );
  await waitFor(() => expect(screen.queryAllByTestId("folder-row")).toHaveLength(2));
  await act(async () => {});
  return { user, dialog: screen.getByTestId("folder-dialog"), closed };
}

function folderRowDeleteButton(path: string): HTMLElement {
  const row = screen
    .getAllByTestId("folder-row")
    .find((r) => within(r).queryByTestId("folder-row-path")?.textContent === path);
  if (!row) {
    throw new Error(`行が見つからない: ${path}`);
  }
  return within(row).getByTestId("folder-row-delete-btn");
}

/** FolderDialog を開き、行 B の登録解除の確認まで開く。closed は確認が閉じたか。 */
async function openFolderDeleteConfirmWithParent() {
  const parent = await openFolder();
  await parent.user.click(folderRowDeleteButton("B"));
  const dialog = await screen.findByTestId("confirm-dialog");
  return {
    user: parent.user,
    dialog,
    closed: () => screen.queryByTestId("confirm-dialog") === null,
    parent: parent.dialog,
    parentClosed: parent.closed,
  };
}

async function openFolderDeleteConfirm(): Promise<Opened> {
  return openFolderDeleteConfirmWithParent();
}

async function openWbImport(): Promise<Opened> {
  const { fn, closed } = closeSpy();
  const { user } = renderWithProviders(
    <WbImportDialog onClose={fn} onImportComplete={() => {}} onThumbnailsReplaced={() => {}} />,
  );
  await waitFor(() => expect(api.listUnmatchedThumbnails).toHaveBeenCalled());
  await act(async () => {});
  return { user, dialog: screen.getByTestId("import-dialog"), closed };
}

async function openTagBarEdit(): Promise<Opened> {
  const { fn, closed } = closeSpy();
  const { user } = renderWithProviders(
    <TagBarEditDialog open={true} onClose={fn} onChanged={() => {}} onTagDeleted={() => {}} />,
  );
  await screen.findAllByTestId("tag-bar-edit-row");
  return { user, dialog: screen.getByTestId("tag-bar-edit-dialog"), closed };
}

async function openUnmatched(): Promise<Opened> {
  const { fn, closed } = closeSpy();
  const { user } = renderWithProviders(
    <UnmatchedThumbnailsDialog onClose={fn} onThumbnailsReplaced={() => {}} />,
  );
  await screen.findByTestId("unmatched-empty");
  return { user, dialog: screen.getByTestId("unmatched-dialog"), closed };
}

// ---- ダイアログの表 ----

interface DialogCase {
  name: string;
  /** `.dialog` の aria-label。ダイアログごとに ModalShell へ渡す label。 */
  ariaLabel: string;
  /** 閉じてよい状態で開く。開いた直後の effect は open() が反映させる。 */
  open: () => Promise<Opened>;
}

// FolderDeleteConfirm は、FolderDialog から開いた登録解除の確認(ConfirmDialog)を指す。
// UnmatchedThumbnailsDialog は単体で描き、WbImportDialog から開いたときは
// 「子のダイアログ」の節で確かめる。
const DIALOGS: DialogCase[] = [
  { name: "AboutDialog", ariaLabel: "バージョン情報", open: openAbout },
  { name: "BulkTagDialog", ariaLabel: "2 件の動画にタグを付ける", open: openBulkTag },
  { name: "ConfirmDialog", ariaLabel: "確認", open: openConfirm },
  { name: "LogViewerDialog", ariaLabel: "動作ログ", open: openLogViewer },
  { name: "SettingsDialog", ariaLabel: "設定", open: openSettings },
  { name: "UpdateDialog", ariaLabel: "更新の確認", open: openUpdate },
  { name: "FolderDialog", ariaLabel: "フォルダ管理", open: openFolder },
  { name: "FolderDeleteConfirm", ariaLabel: "フォルダの登録を解除", open: openFolderDeleteConfirm },
  { name: "WbImportDialog", ariaLabel: ".wbインポート", open: openWbImport },
  { name: "TagBarEditDialog", ariaLabel: "タグの編集", open: openTagBarEdit },
  {
    name: "UnmatchedThumbnailsDialog",
    ariaLabel: "自動対応できなかった旧サムネイル",
    open: openUnmatched,
  },
];

/** 開いて、開いた直後の effect(初期フォーカスなど)を反映させてから返す。 */
async function open(dialog: DialogCase): Promise<Opened> {
  const opened = await dialog.open();
  await act(async () => {});
  return opened;
}

// ---- 属性と描画の場所 ----

test.each(DIALOGS)(
  "$name は ModalShell を通り、body 直下の背景の中に、label を aria-label にした dialog として描かれる",
  async (c) => {
    const { dialog } = await open(c);
    const overlay = overlayOf(dialog);

    expect(dialog).toHaveClass("dialog");
    expect(dialog.getAttribute("role")).toBe("dialog");
    expect(dialog.getAttribute("aria-modal")).toBe("true");
    expect(dialog.getAttribute("aria-label")).toBe(c.ariaLabel);
    expect(overlay.parentElement).toBe(document.body);
  },
);

// ---- 閉じてよい状態での閉じ方 ----

test.each(DIALOGS)("$name は処理中でなければ背景のクリックで閉じる", async (c) => {
  const { user, dialog, closed } = await open(c);

  await user.click(overlayOf(dialog));

  expect(closed()).toBe(true);
});

test.each(DIALOGS)("$name は処理中でなければ Escape で閉じる", async (c) => {
  const { user, closed } = await open(c);

  await user.keyboard("{Escape}");

  expect(closed()).toBe(true);
});

test("SettingsDialog は設定の読み込み中でも、背景のクリックと Escape で閉じる", async () => {
  vi.mocked(api.getNasPollIntervalMinutes).mockReturnValue(never());
  const onClose = vi.fn();
  const { user } = renderWithProviders(<SettingsDialog open={true} onClose={onClose} />);
  expect(screen.getByTestId("settings-dialog-save-btn")).toBeDisabled();

  await user.click(overlayOf(screen.getByTestId("settings-dialog")));
  expect(onClose).toHaveBeenCalledTimes(1);

  await user.keyboard("{Escape}");
  expect(onClose).toHaveBeenCalledTimes(2);
});

test("UpdateDialog は確認中でも、背景のクリックと Escape で閉じる", async () => {
  vi.mocked(api.checkForUpdate).mockReturnValue(never());
  const onClose = vi.fn();
  const { user } = renderWithProviders(<UpdateDialog open={true} onClose={onClose} />);
  expect(screen.getByTestId("update-dialog-checking")).toBeInTheDocument();

  await user.click(overlayOf(screen.getByTestId("update-dialog")));
  expect(onClose).toHaveBeenCalledTimes(1);

  await user.keyboard("{Escape}");
  expect(onClose).toHaveBeenCalledTimes(2);
});

test("UpdateDialog は確認に失敗した状態でも Escape で閉じる", async () => {
  vi.spyOn(console, "error").mockImplementation(() => {});
  vi.mocked(api.checkForUpdate).mockRejectedValue("失敗");
  const onClose = vi.fn();
  const { user } = renderWithProviders(<UpdateDialog open={true} onClose={onClose} />);
  await screen.findByTestId("update-dialog-error");

  await user.keyboard("{Escape}");

  expect(onClose).toHaveBeenCalledTimes(1);
});

// ---- 処理中は閉じない ----

interface BusyCase {
  name: string;
  /** 処理中の状態まで進める。overlays は背景のクリックを試す対象(自分と、開いている子)。 */
  enter: () => Promise<{
    user: UserEvent;
    overlays: HTMLElement[];
    /** 閉じる操作が届いておらず、ダイアログが残っていること。 */
    assertStillOpen: () => void;
  }>;
}

const BUSY: BusyCase[] = [
  {
    name: "ConfirmDialog(busy)",
    enter: async () => {
      const { user, dialog, closed } = renderConfirm(true);
      return {
        user,
        overlays: [overlayOf(dialog)],
        assertStillOpen: () => expect(closed()).toBe(false),
      };
    },
  },
  {
    name: "BulkTagDialog(付与の処理中)",
    enter: async () => {
      vi.mocked(api.assignTagsToVideos).mockReturnValue(never());
      const { user, dialog, closed } = await openBulkTag();
      await user.type(screen.getByTestId("bulk-tag-input"), "タグ");
      await user.click(screen.getByTestId("bulk-tag-submit-btn"));
      await waitFor(() => expect(screen.getByTestId("bulk-tag-cancel-btn")).toBeDisabled());
      return {
        user,
        overlays: [overlayOf(dialog)],
        assertStillOpen: () => expect(closed()).toBe(false),
      };
    },
  },
  {
    name: "SettingsDialog(保存中)",
    enter: async () => {
      vi.mocked(api.setNasPollIntervalMinutes).mockReturnValue(never());
      const { user, dialog, closed } = await openSettings();
      await user.click(screen.getByTestId("settings-dialog-save-btn"));
      await waitFor(() => expect(screen.getByTestId("settings-dialog-save-btn")).toBeDisabled());
      return {
        user,
        overlays: [overlayOf(dialog)],
        assertStillOpen: () => expect(closed()).toBe(false),
      };
    },
  },
  {
    name: "UpdateDialog(ダウンロード中)",
    enter: async () => {
      const { user, dialog, closed } = await openUpdate();
      await user.click(screen.getByTestId("update-dialog-apply-btn"));
      await screen.findByTestId("update-dialog-progress");
      return {
        user,
        overlays: [overlayOf(dialog)],
        assertStillOpen: () => expect(closed()).toBe(false),
      };
    },
  },
  {
    name: "FolderDeleteConfirm(解除中。親の FolderDialog も閉じない)",
    enter: async () => {
      vi.mocked(api.removeWatchFolder).mockReturnValue(never());
      const { user, dialog, closed, parentClosed } = await openFolderDeleteConfirmWithParent();
      await user.click(screen.getByTestId("confirm-dialog-confirm-btn"));
      await waitFor(() => expect(screen.getByTestId("confirm-dialog-confirm-btn")).toBeDisabled());
      return {
        user,
        overlays: [overlayOf(dialog)],
        assertStillOpen: () => {
          expect(closed()).toBe(false);
          expect(parentClosed()).toBe(false);
        },
      };
    },
  },
  {
    name: "TagBarEditDialog(並び替えの保存中)",
    enter: async () => {
      vi.mocked(api.getTagLayout).mockResolvedValue([
        { kind: "tag", id: 1, name: "猫", video_count: 2 },
        { kind: "tag", id: 2, name: "犬", video_count: 1 },
      ]);
      vi.mocked(api.moveTagLayoutItem).mockReturnValue(never());
      const { user, dialog, closed } = await openTagBarEdit();
      await user.click(screen.getAllByTestId("tag-bar-edit-move-down-btn")[0]);
      await waitFor(() =>
        expect(screen.getAllByTestId("tag-bar-edit-move-down-btn")[0]).toBeDisabled(),
      );
      return {
        user,
        overlays: [overlayOf(dialog)],
        assertStillOpen: () => {
          expect(closed()).toBe(false);
          expect(dialog).toBeInTheDocument();
        },
      };
    },
  },
  {
    name: "TagBarEditDialog(削除中。削除の確認も閉じない)",
    enter: async () => {
      vi.mocked(api.deleteTag).mockReturnValue(never());
      const { user, dialog, closed } = await openTagBarEdit();
      await user.click(screen.getByTestId("tag-bar-edit-delete-btn"));
      await user.click(screen.getByTestId("confirm-dialog-confirm-btn"));
      await waitFor(() => expect(screen.getByTestId("confirm-dialog-confirm-btn")).toBeDisabled());
      const confirm = screen.getByTestId("confirm-dialog");
      return {
        user,
        overlays: [overlayOf(dialog), overlayOf(confirm)],
        assertStillOpen: () => {
          expect(closed()).toBe(false);
          expect(confirm).toBeInTheDocument();
        },
      };
    },
  },
  {
    name: "WbImportDialog(インポート中)",
    enter: async () => {
      vi.mocked(api.startWbImport).mockReturnValue(never());
      const { user, dialog, closed } = await openWbImport();
      await user.click(screen.getByRole("button", { name: ".wbファイルを選択" }));
      await user.click(screen.getByRole("button", { name: "旧サムネイルフォルダを選択" }));
      await user.click(screen.getByRole("button", { name: "インポート" }));
      await waitFor(() => expect(screen.getByTestId("import-dialog-close-btn")).toBeDisabled());
      return {
        user,
        overlays: [overlayOf(dialog)],
        assertStillOpen: () => {
          expect(closed()).toBe(false);
          expect(dialog).toBeInTheDocument();
        },
      };
    },
  },
];

test.each(BUSY)("$name は、背景のクリックでも Escape でも閉じない", async ({ enter }) => {
  const { user, overlays, assertStillOpen } = await enter();

  for (const overlay of overlays) {
    await user.click(overlay);
  }
  await user.keyboard("{Escape}");

  assertStillOpen();
});

// ---- 開いていないとき ----

// 開いていない状態では何も描かず、Escape が来ても閉じる操作を届けない。
const NOT_OPEN: { name: string; testId: string; render: (onClose: () => void) => ReactElement }[] = [
  {
    name: "ConfirmDialog(open=false)",
    testId: "confirm-dialog",
    render: (onClose) => (
      <ConfirmDialog
        open={false}
        title="確認"
        confirmLabel="実行"
        onConfirm={() => {}}
        onCancel={onClose}
      >
        本文
      </ConfirmDialog>
    ),
  },
  {
    name: "SettingsDialog(open=false)",
    testId: "settings-dialog",
    render: (onClose) => <SettingsDialog open={false} onClose={onClose} />,
  },
  {
    name: "UpdateDialog(open=false)",
    testId: "update-dialog",
    render: (onClose) => <UpdateDialog open={false} onClose={onClose} />,
  },
  {
    name: "AboutDialog(open=false)",
    testId: "about-dialog",
    render: (onClose) => <AboutDialog open={false} onClose={onClose} />,
  },
  {
    name: "LogViewerDialog(open=false)",
    testId: "log-viewer-dialog",
    render: (onClose) => <LogViewerDialog open={false} onClose={onClose} />,
  },
  {
    name: "BulkTagDialog(videoIds=null)",
    testId: "bulk-tag-dialog",
    render: (onClose) => <BulkTagDialog videoIds={null} onTagged={() => {}} onClose={onClose} />,
  },
];

test.each(NOT_OPEN)("$name は描画されず、Escape でも閉じる操作を届けない", async ({ testId, render }) => {
  const onClose = vi.fn();
  const { user } = renderWithProviders(render(onClose));

  await user.keyboard("{Escape}");

  expect(screen.queryByTestId(testId)).toBeNull();
  expect(onClose).not.toHaveBeenCalled();
});

// ---- 子のダイアログ ----
// 子が開いている間、背景のクリックも Escape も子だけに効き、親には届かない。
// 子が閉じたあとの Escape で親が閉じる。

interface ChildCase {
  pair: string;
  open: () => Promise<{
    user: UserEvent;
    parent: HTMLElement;
    child: HTMLElement;
    parentClosed: () => boolean;
    /** 子を閉じても、確認の対象(解除・削除)は実行されていないこと。 */
    assertNothingExecuted: () => void;
  }>;
}

const CHILDREN: ChildCase[] = [
  {
    pair: "FolderDialog → 登録解除の確認",
    open: async () => {
      const { user, dialog, parent, parentClosed } = await openFolderDeleteConfirmWithParent();
      return {
        user,
        parent,
        child: dialog,
        parentClosed,
        assertNothingExecuted: () => expect(api.removeWatchFolder).not.toHaveBeenCalled(),
      };
    },
  },
  {
    pair: "TagBarEditDialog → 削除の確認",
    open: async () => {
      const { user, dialog, closed } = await openTagBarEdit();
      await user.click(screen.getByTestId("tag-bar-edit-delete-btn"));
      return {
        user,
        parent: dialog,
        child: screen.getByTestId("confirm-dialog"),
        parentClosed: closed,
        assertNothingExecuted: () => expect(api.deleteTag).not.toHaveBeenCalled(),
      };
    },
  },
  {
    pair: "WbImportDialog → UnmatchedThumbnailsDialog",
    open: async () => {
      vi.mocked(api.listUnmatchedThumbnails).mockResolvedValue([UNMATCHED_ROW]);
      const { user, dialog, closed } = await openWbImport();
      await user.click(await screen.findByTestId("import-unmatched-open-btn"));
      await screen.findByTestId("unmatched-row");
      return {
        user,
        parent: dialog,
        child: screen.getByTestId("unmatched-dialog"),
        parentClosed: closed,
        assertNothingExecuted: () => {},
      };
    },
  },
];

test.each(CHILDREN)(
  "$pair の子は、Escape で子だけが閉じ、次の Escape で親が閉じる",
  async ({ open: openPair }) => {
    const { user, parent, child, parentClosed, assertNothingExecuted } = await openPair();

    await user.keyboard("{Escape}");

    expect(child).not.toBeInTheDocument();
    expect(parent).toBeInTheDocument();
    expect(parentClosed()).toBe(false);
    assertNothingExecuted();

    await user.keyboard("{Escape}");

    expect(parentClosed()).toBe(true);
  },
);

test.each(CHILDREN)("$pair の子は、中をクリックしても閉じず、親も閉じない", async ({ open: openPair }) => {
  const { user, parent, child, parentClosed } = await openPair();

  await user.click(within(child).getByRole("heading"));
  await user.click(child);

  expect(child).toBeInTheDocument();
  expect(parent).toBeInTheDocument();
  expect(parentClosed()).toBe(false);
});

test.each(CHILDREN)("$pair の子は、背景のクリックで子だけが閉じる", async ({ open: openPair }) => {
  const { user, parent, child, parentClosed, assertNothingExecuted } = await openPair();

  await user.click(overlayOf(child));

  expect(child).not.toBeInTheDocument();
  expect(parent).toBeInTheDocument();
  expect(parentClosed()).toBe(false);
  assertNothingExecuted();
});

// 背景はどれも同じ z-index なので、重なりは DOM の順で決まる。子が親の上に出るには、
// 子の背景が親の背景より後ろにある必要がある。
test.each(CHILDREN)("$pair の子の背景は、親の背景より DOM の後ろに描かれる", async ({ open: openPair }) => {
  const { parent, child } = await openPair();

  expect(
    overlayOf(parent).compareDocumentPosition(overlayOf(child)) & Node.DOCUMENT_POSITION_FOLLOWING,
  ).toBeTruthy();
});

// ---- 初期フォーカス ----
// 移さないダイアログ(FolderDialog・WbImportDialog・TagBarEditDialog・
// UnmatchedThumbnailsDialog)は、body のままであることを固定しない。

interface FocusExpectation {
  name: string;
  open: () => Promise<Opened>;
  /** 開いた直後に document.activeElement であるべき要素の説明。 */
  description: string;
  target: () => Element;
}

const FOCUS: FocusExpectation[] = [
  {
    name: "AboutDialog",
    open: openAbout,
    description: "閉じるボタン",
    target: () => screen.getByTestId("about-dialog-close-btn"),
  },
  {
    name: "BulkTagDialog",
    open: openBulkTag,
    description: "タグ名の入力欄",
    target: () => screen.getByTestId("bulk-tag-input"),
  },
  {
    name: "ConfirmDialog",
    open: openConfirm,
    description: "キャンセルボタン",
    target: () => screen.getByTestId("confirm-dialog-cancel-btn"),
  },
  {
    name: "LogViewerDialog",
    open: openLogViewer,
    description: "閉じるボタン",
    target: () => screen.getByTestId("log-viewer-close-btn"),
  },
  {
    name: "SettingsDialog",
    open: openSettings,
    description: "設定の読み込み後の確認間隔の入力欄",
    target: () => screen.getByTestId("settings-nas-interval-input"),
  },
  {
    name: "UpdateDialog",
    open: openUpdate,
    description: "更新の案内の主ボタン(今すぐ更新)",
    target: () => screen.getByTestId("update-dialog-apply-btn"),
  },
  {
    name: "FolderDeleteConfirm",
    open: openFolderDeleteConfirm,
    description: "キャンセルボタン",
    target: () => screen.getByTestId("confirm-dialog-cancel-btn"),
  },
];

test.each(FOCUS)("$name は開いた直後のフォーカスが $description", async ({ open: openIt, target }) => {
  await openIt();
  await act(async () => {});

  expect(document.activeElement).toBe(target());
});

test("SettingsDialog は設定の読み込み中はフォーカスを移さない(body のまま)", async () => {
  vi.mocked(api.getNasPollIntervalMinutes).mockReturnValue(never());
  renderWithProviders(<SettingsDialog open={true} onClose={() => {}} />);
  await act(async () => {});

  expect(document.activeElement).toBe(document.body);
});

test("UpdateDialog は確認中は主ボタンが無く、フォーカスを移さない(body のまま)", async () => {
  vi.mocked(api.checkForUpdate).mockReturnValue(never());
  renderWithProviders(<UpdateDialog open={true} onClose={() => {}} />);
  expect(screen.getByTestId("update-dialog-checking")).toBeInTheDocument();
  await act(async () => {});

  expect(document.activeElement).toBe(document.body);
});

// ---- 入力欄の編集メニュー ----
// 編集メニューの Escape はダイアログの閉じ方とは別の仕様なので、表には含めない。

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

// ---- ModalShell の規則 ----

function renderShell(props: { onDismiss: () => void; dismissible?: boolean }) {
  const view = renderWithProviders(
    <ModalShell label="殻" className="shell" testId="shell" {...props}>
      <p>本文</p>
    </ModalShell>,
  );
  return { ...view, overlay: overlayOf(screen.getByTestId("shell")) };
}

test("ModalShell は role=dialog・aria-modal・aria-label を持つ .dialog を描く", () => {
  renderShell({ onDismiss: () => {} });
  const dialog = screen.getByTestId("shell");

  expect(dialog).toHaveClass("dialog");
  expect(dialog.getAttribute("role")).toBe("dialog");
  expect(dialog.getAttribute("aria-modal")).toBe("true");
  expect(dialog.getAttribute("aria-label")).toBe("殻");
});

test("ModalShell は背景を render の親要素の外(body 直下)に portal で描く", () => {
  const { container, overlay } = renderShell({ onDismiss: () => {} });

  expect(overlay.parentElement).toBe(document.body);
  expect(container.contains(overlay)).toBe(false);
});

test("ModalShell は .dialog の中(本文・.dialog 自体)をクリックしても onDismiss を呼ばない", async () => {
  const onDismiss = vi.fn();
  const { user } = renderShell({ onDismiss });

  await user.click(screen.getByText("本文"));
  await user.click(screen.getByTestId("shell"));

  expect(onDismiss).not.toHaveBeenCalled();
});

test("ModalShell は dismissible が既定(true)なら、背景のクリックと Escape のどちらでも onDismiss を呼ぶ", async () => {
  const onDismiss = vi.fn();
  const { user, overlay } = renderShell({ onDismiss });

  await user.click(overlay);
  expect(onDismiss).toHaveBeenCalledTimes(1);

  await user.keyboard("{Escape}");
  expect(onDismiss).toHaveBeenCalledTimes(2);
});

test("ModalShell は dismissible が false なら、背景のクリックでも Escape でも onDismiss を呼ばない", async () => {
  const onDismiss = vi.fn();
  const { user, overlay } = renderShell({ onDismiss, dismissible: false });

  await user.click(overlay);
  await user.keyboard("{Escape}");

  expect(onDismiss).not.toHaveBeenCalled();
});

test("ModalShell は dismissible が false の間だけ「閉じられないモーダルがある」状態にし、アンマウントで戻す", () => {
  expect(hasUndismissibleModal()).toBe(false);

  const { rerender, unmount } = renderShell({ onDismiss: () => {}, dismissible: false });
  expect(hasUndismissibleModal()).toBe(true);

  rerender(
    <ModalShell label="殻" className="shell" testId="shell" onDismiss={() => {}} dismissible>
      <p>本文</p>
    </ModalShell>,
  );
  expect(hasUndismissibleModal()).toBe(false);

  rerender(
    <ModalShell label="殻" className="shell" testId="shell" onDismiss={() => {}} dismissible={false}>
      <p>本文</p>
    </ModalShell>,
  );
  expect(hasUndismissibleModal()).toBe(true);

  unmount();
  expect(hasUndismissibleModal()).toBe(false);
});
