// 各ダイアログの「外側」(背景のクリック・中のクリック・子のダイアログ・初期フォーカス・
// role/aria 属性・描画の場所)の現状を固定する特性テスト。Escape の扱いは
// escapeToClose.test.tsx が固定している。
// 属性と描画の場所の期待値は表にまとめてあり、仕様を変えるときは表の該当行を更新する。
import { beforeEach, expect, test, vi } from "vitest";
import { act, screen, waitFor, within } from "@testing-library/react";
import type { UserEvent } from "@testing-library/user-event";
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

function fakeUpdate(downloadAndInstall: () => Promise<void>) {
  return {
    version: "2.0.0",
    currentVersion: "1.0.0",
    body: "ノート",
    close: vi.fn().mockResolvedValue(undefined),
    downloadAndInstall: vi.fn(downloadAndInstall),
  } as unknown as NonNullable<Awaited<ReturnType<typeof api.checkForUpdate>>>;
}

const UNMATCHED_ROW = {
  folder_path: "C:\\old",
  file_name: "a.jpg",
  kind: "no_matching_video" as const,
  default_search_term: "a",
};

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
  vi.mocked(api.listTagsWithVideoCount).mockResolvedValue([{ id: 1, name: "猫", video_count: 2 }]);
  vi.mocked(api.pickWbFile).mockResolvedValue("C:\\a.wb");
  vi.mocked(api.pickWbThumbnailFolder).mockResolvedValue("C:\\thumbs");
});

// ---- 開き方 ----

type DialogName =
  | "AboutDialog"
  | "BulkTagDialog"
  | "ConfirmDialog"
  | "LogViewerDialog"
  | "SettingsDialog"
  | "UpdateDialog"
  | "FolderDialog"
  | "FolderDeleteConfirm"
  | "WbImportDialog"
  | "TagBarEditDialog"
  | "UnmatchedThumbnailsDialog";

interface Opened {
  user: UserEvent;
  /** render が描いた親要素。portal しないダイアログはこの中に描かれる。 */
  container: HTMLElement;
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
  const { user, container } = renderWithProviders(<AboutDialog open={true} onClose={fn} />);
  await screen.findByText("バージョン 1.2.3");
  return { user, container, dialog: screen.getByTestId("about-dialog"), closed };
}

async function openBulkTag(): Promise<Opened> {
  const { fn, closed } = closeSpy();
  const { user, container } = renderWithProviders(
    <BulkTagDialog videoIds={["a", "b"]} onTagged={() => {}} onClose={fn} />,
  );
  await waitFor(() => expect(api.listAllTags).toHaveBeenCalled());
  await act(async () => {});
  return { user, container, dialog: screen.getByTestId("bulk-tag-dialog"), closed };
}

function renderConfirm(busy: boolean): Opened {
  const { fn, closed } = closeSpy();
  const { user, container } = renderWithProviders(
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
  return { user, container, dialog: screen.getByTestId("confirm-dialog"), closed };
}

async function openConfirm(): Promise<Opened> {
  return renderConfirm(false);
}

async function openLogViewer(): Promise<Opened> {
  const { fn, closed } = closeSpy();
  const { user, container } = renderWithProviders(<LogViewerDialog open={true} onClose={fn} />);
  await waitFor(() => expect(api.readLog).toHaveBeenCalled());
  await screen.findByText("ログフォルダ: C:\\logs");
  return { user, container, dialog: screen.getByTestId("log-viewer-dialog"), closed };
}

async function openSettings(): Promise<Opened> {
  const { fn, closed } = closeSpy();
  const { user, container } = renderWithProviders(<SettingsDialog open={true} onClose={fn} />);
  await waitFor(() => expect(screen.getByTestId("settings-nas-interval-input")).toBeEnabled());
  return { user, container, dialog: screen.getByTestId("settings-dialog"), closed };
}

/** 更新の案内(available)が出た状態まで開く。 */
async function openUpdate(): Promise<Opened> {
  const { fn, closed } = closeSpy();
  const { user, container } = renderWithProviders(<UpdateDialog open={true} onClose={fn} />);
  await screen.findByTestId("update-dialog-available");
  return { user, container, dialog: screen.getByTestId("update-dialog"), closed };
}

async function openFolder(): Promise<Opened> {
  const { fn, closed } = closeSpy();
  const { user, container } = renderWithProviders(
    <FolderDialog open={true} onClose={fn} onChanged={() => {}} />,
  );
  await waitFor(() => expect(screen.queryAllByTestId("folder-row")).toHaveLength(2));
  await act(async () => {});
  return { user, container, dialog: screen.getByTestId("folder-dialog"), closed };
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
  const dialog = await screen.findByTestId("folder-delete-confirm-dialog");
  return {
    user: parent.user,
    container: parent.container,
    dialog,
    closed: () => screen.queryByTestId("folder-delete-confirm-dialog") === null,
    parentClosed: parent.closed,
  };
}

async function openFolderDeleteConfirm(): Promise<Opened> {
  return openFolderDeleteConfirmWithParent();
}

async function openWbImport(): Promise<Opened> {
  const { fn, closed } = closeSpy();
  const { user, container } = renderWithProviders(
    <WbImportDialog onClose={fn} onImportComplete={() => {}} onThumbnailsReplaced={() => {}} />,
  );
  await waitFor(() => expect(api.listUnmatchedThumbnails).toHaveBeenCalled());
  await act(async () => {});
  return { user, container, dialog: screen.getByTestId("import-dialog"), closed };
}

async function openTagBarEdit(): Promise<Opened> {
  const { fn, closed } = closeSpy();
  const { user, container } = renderWithProviders(
    <TagBarEditDialog open={true} onClose={fn} onChanged={() => {}} onTagDeleted={() => {}} />,
  );
  await screen.findByTestId("tag-bar-edit-row");
  return { user, container, dialog: screen.getByTestId("tag-bar-edit-dialog"), closed };
}

async function openUnmatched(): Promise<Opened> {
  const { fn, closed } = closeSpy();
  const { user, container } = renderWithProviders(
    <UnmatchedThumbnailsDialog onClose={fn} onThumbnailsReplaced={() => {}} />,
  );
  await screen.findByTestId("unmatched-empty");
  return { user, container, dialog: screen.getByTestId("unmatched-dialog"), closed };
}

// FolderDeleteConfirm は FolderDialog から開いた状態で確かめる(単体では描かない)。
// UnmatchedThumbnailsDialog は単体で描き、WbImportDialog から開いたときは
// 「子のダイアログ」の節で確かめる。
const OPEN: Record<DialogName, () => Promise<Opened>> = {
  AboutDialog: openAbout,
  BulkTagDialog: openBulkTag,
  ConfirmDialog: openConfirm,
  LogViewerDialog: openLogViewer,
  SettingsDialog: openSettings,
  UpdateDialog: openUpdate,
  FolderDialog: openFolder,
  FolderDeleteConfirm: openFolderDeleteConfirm,
  WbImportDialog: openWbImport,
  TagBarEditDialog: openTagBarEdit,
  UnmatchedThumbnailsDialog: openUnmatched,
};

const ALL_DIALOGS = Object.keys(OPEN) as DialogName[];

/** 開いて、開いた直後の effect(初期フォーカスなど)を反映させてから返す。 */
async function open(name: DialogName): Promise<Opened> {
  const opened = await OPEN[name]();
  await act(async () => {});
  return opened;
}

// ---- 属性 ----

interface AttributeExpectation {
  name: DialogName;
  /** null は属性が無いこと。 */
  role: string | null;
  ariaModal: string | null;
  ariaLabel: string | null;
}

const ATTRIBUTES: AttributeExpectation[] = [
  { name: "AboutDialog", role: "dialog", ariaModal: "true", ariaLabel: "バージョン情報" },
  {
    name: "BulkTagDialog",
    role: "dialog",
    ariaModal: "true",
    ariaLabel: "2 件の動画にタグを付ける",
  },
  { name: "ConfirmDialog", role: "dialog", ariaModal: "true", ariaLabel: "確認" },
  { name: "LogViewerDialog", role: "dialog", ariaModal: "true", ariaLabel: "動作ログ" },
  { name: "SettingsDialog", role: "dialog", ariaModal: "true", ariaLabel: "設定" },
  { name: "UpdateDialog", role: "dialog", ariaModal: "true", ariaLabel: "更新の確認" },
  { name: "FolderDialog", role: "dialog", ariaModal: "true", ariaLabel: "フォルダ管理" },
  {
    name: "FolderDeleteConfirm",
    role: "dialog",
    ariaModal: "true",
    ariaLabel: "フォルダの登録を解除",
  },
  { name: "WbImportDialog", role: "dialog", ariaModal: "true", ariaLabel: ".wbインポート" },
  { name: "TagBarEditDialog", role: "dialog", ariaModal: "true", ariaLabel: "タグの編集" },
  {
    name: "UnmatchedThumbnailsDialog",
    role: "dialog",
    ariaModal: "true",
    ariaLabel: "自動対応できなかった旧サムネイル",
  },
];

test("属性の表は全ダイアログを1回ずつ含む", () => {
  expect(ATTRIBUTES.map((a) => a.name).sort()).toEqual([...ALL_DIALOGS].sort());
});

test.each(ATTRIBUTES)(
  "$name の .dialog の role=$role aria-modal=$ariaModal aria-label=$ariaLabel",
  async ({ name, role, ariaModal, ariaLabel }) => {
    const { dialog } = await open(name);

    expect(dialog).toHaveClass("dialog");
    expect(dialog.getAttribute("role")).toBe(role);
    expect(dialog.getAttribute("aria-modal")).toBe(ariaModal);
    expect(dialog.getAttribute("aria-label")).toBe(ariaLabel);
  },
);

// ---- 描画の場所 ----

interface PlacementExpectation {
  name: DialogName;
  /** body: document.body の直下に portal で描く。container: render の親要素の中に描く。 */
  placement: "body" | "container";
}

const PLACEMENTS: PlacementExpectation[] = [
  { name: "AboutDialog", placement: "body" },
  { name: "BulkTagDialog", placement: "body" },
  { name: "ConfirmDialog", placement: "body" },
  { name: "LogViewerDialog", placement: "body" },
  { name: "SettingsDialog", placement: "body" },
  { name: "UpdateDialog", placement: "body" },
  { name: "FolderDialog", placement: "body" },
  { name: "FolderDeleteConfirm", placement: "body" },
  { name: "WbImportDialog", placement: "body" },
  { name: "TagBarEditDialog", placement: "body" },
  { name: "UnmatchedThumbnailsDialog", placement: "body" },
];

test("描画の場所の表は全ダイアログを1回ずつ含む", () => {
  expect(PLACEMENTS.map((p) => p.name).sort()).toEqual([...ALL_DIALOGS].sort());
});

test.each(PLACEMENTS)("$name の .dialog-overlay は $placement の直下に描かれる", async ({ name, placement }) => {
  const { container, dialog } = await open(name);
  const overlay = overlayOf(dialog);

  if (placement === "body") {
    expect(overlay.parentElement).toBe(document.body);
    expect(container.contains(overlay)).toBe(false);
  } else {
    expect(overlay.parentElement).toBe(container);
  }
});

// ---- 初期フォーカス ----

interface FocusExpectation {
  name: DialogName;
  /** 開いた直後に document.activeElement であるべき要素の説明。 */
  description: string;
  target: () => Element;
}

const FOCUS: FocusExpectation[] = [
  {
    name: "AboutDialog",
    description: "閉じるボタン",
    target: () => screen.getByTestId("about-dialog-close-btn"),
  },
  {
    name: "BulkTagDialog",
    description: "タグ名の入力欄",
    target: () => screen.getByTestId("bulk-tag-input"),
  },
  {
    name: "ConfirmDialog",
    description: "キャンセルボタン",
    target: () => screen.getByTestId("confirm-dialog-cancel-btn"),
  },
  {
    name: "LogViewerDialog",
    description: "閉じるボタン",
    target: () => screen.getByTestId("log-viewer-close-btn"),
  },
  {
    name: "SettingsDialog",
    description: "設定の読み込み後の確認間隔の入力欄",
    target: () => screen.getByTestId("settings-nas-interval-input"),
  },
  {
    name: "UpdateDialog",
    description: "更新の案内の主ボタン(今すぐ更新)",
    target: () => screen.getByTestId("update-dialog-apply-btn"),
  },
  { name: "FolderDialog", description: "body(移さない)", target: () => document.body },
  {
    name: "FolderDeleteConfirm",
    description: "押した行の登録解除ボタンのまま(移さない)",
    target: () => folderRowDeleteButton("B"),
  },
  { name: "WbImportDialog", description: "body(移さない)", target: () => document.body },
  { name: "TagBarEditDialog", description: "body(移さない)", target: () => document.body },
  {
    name: "UnmatchedThumbnailsDialog",
    description: "body(移さない)",
    target: () => document.body,
  },
];

test("初期フォーカスの表は全ダイアログを1回ずつ含む", () => {
  expect(FOCUS.map((f) => f.name).sort()).toEqual([...ALL_DIALOGS].sort());
});

test.each(FOCUS)("$name は開いた直後のフォーカスが $description", async ({ name, target }) => {
  await open(name);

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

// ---- ダイアログの中のクリック ----

test.each(ALL_DIALOGS)("%s は .dialog の中(見出し・本文・.dialog 自体)をクリックしても閉じない", async (name) => {
  const { user, dialog, closed } = await open(name);
  const firstText = dialog.querySelector("h2, p");
  expect(firstText).not.toBeNull();

  await user.click(firstText!);
  await user.click(dialog);

  expect(closed()).toBe(false);
  expect(dialog).toBeInTheDocument();
});

// ---- 背景のクリック ----

test.each(ALL_DIALOGS)("%s は処理中でなければ背景のクリックで閉じる", async (name) => {
  const { user, dialog, closed } = await open(name);

  await user.click(overlayOf(dialog));

  expect(closed()).toBe(true);
});

test("ConfirmDialog は busy の間は背景をクリックしても閉じない", async () => {
  const { user, dialog, closed } = renderConfirm(true);

  await user.click(overlayOf(dialog));

  expect(closed()).toBe(false);
});

test("BulkTagDialog は付与の処理中は背景をクリックしても閉じない", async () => {
  vi.mocked(api.assignTagToVideos).mockReturnValue(never());
  const { user, dialog, closed } = await openBulkTag();
  await user.type(screen.getByTestId("bulk-tag-input"), "タグ");
  await user.click(screen.getByTestId("bulk-tag-submit-btn"));
  await waitFor(() => expect(screen.getByTestId("bulk-tag-cancel-btn")).toBeDisabled());

  await user.click(overlayOf(dialog));

  expect(closed()).toBe(false);
});

test("SettingsDialog は保存中は背景をクリックしても閉じない", async () => {
  vi.mocked(api.setNasPollIntervalMinutes).mockReturnValue(never());
  const { user, dialog, closed } = await openSettings();
  await user.click(screen.getByTestId("settings-dialog-save-btn"));
  await waitFor(() => expect(screen.getByTestId("settings-dialog-save-btn")).toBeDisabled());

  await user.click(overlayOf(dialog));

  expect(closed()).toBe(false);
});

test("SettingsDialog は設定の読み込み中でも背景のクリックで閉じる", async () => {
  vi.mocked(api.getNasPollIntervalMinutes).mockReturnValue(never());
  const onClose = vi.fn();
  const { user } = renderWithProviders(<SettingsDialog open={true} onClose={onClose} />);
  expect(screen.getByTestId("settings-dialog-save-btn")).toBeDisabled();

  await user.click(overlayOf(screen.getByTestId("settings-dialog")));

  expect(onClose).toHaveBeenCalledTimes(1);
});

test("UpdateDialog は確認中でも背景のクリックで閉じる", async () => {
  vi.mocked(api.checkForUpdate).mockReturnValue(never());
  const onClose = vi.fn();
  const { user } = renderWithProviders(<UpdateDialog open={true} onClose={onClose} />);
  expect(screen.getByTestId("update-dialog-checking")).toBeInTheDocument();

  await user.click(overlayOf(screen.getByTestId("update-dialog")));

  expect(onClose).toHaveBeenCalledTimes(1);
});

test("UpdateDialog はダウンロード中(閉じられない状態)は背景をクリックしても閉じない", async () => {
  const { user, dialog, closed } = await openUpdate();
  await user.click(screen.getByTestId("update-dialog-apply-btn"));
  await screen.findByTestId("update-dialog-progress");

  await user.click(overlayOf(dialog));

  expect(closed()).toBe(false);
});

test("FolderDeleteConfirm は解除中は背景をクリックしても閉じず、親の FolderDialog も閉じない", async () => {
  vi.mocked(api.removeWatchFolder).mockReturnValue(never());
  const { user, dialog, closed, parentClosed } = await openFolderDeleteConfirmWithParent();
  await user.click(screen.getByTestId("folder-delete-confirm-btn"));
  await waitFor(() => expect(screen.getByTestId("folder-delete-confirm-btn")).toBeDisabled());

  await user.click(overlayOf(dialog));

  expect(closed()).toBe(false);
  expect(parentClosed()).toBe(false);
});

test("TagBarEditDialog は保存中は背景をクリックしても閉じない", async () => {
  vi.mocked(api.setTagOrder).mockReturnValue(never());
  const { user, dialog, closed } = await openTagBarEdit();
  await user.click(screen.getByTestId("tag-bar-edit-save-btn"));
  await waitFor(() => expect(screen.getByTestId("tag-bar-edit-save-btn")).toBeDisabled());

  await user.click(overlayOf(dialog));

  expect(closed()).toBe(false);
});

test("TagBarEditDialog は削除中は自分の背景をクリックしても閉じず、削除の確認も背景のクリックで閉じない", async () => {
  vi.mocked(api.deleteTag).mockReturnValue(never());
  const { user, dialog, closed } = await openTagBarEdit();
  await user.click(screen.getByTestId("tag-bar-edit-delete-btn"));
  await user.click(screen.getByTestId("confirm-dialog-confirm-btn"));
  await waitFor(() => expect(screen.getByTestId("confirm-dialog-confirm-btn")).toBeDisabled());

  await user.click(overlayOf(dialog));
  await user.click(overlayOf(screen.getByTestId("confirm-dialog")));

  expect(closed()).toBe(false);
  expect(screen.getByTestId("confirm-dialog")).toBeInTheDocument();
});

test("WbImportDialog はインポート中は背景をクリックしても閉じない", async () => {
  vi.mocked(api.startWbImport).mockReturnValue(never());
  const { user, dialog, closed } = await openWbImport();
  await user.click(screen.getByRole("button", { name: ".wbファイルを選択" }));
  await user.click(screen.getByRole("button", { name: "旧サムネイルフォルダを選択" }));
  await user.click(screen.getByRole("button", { name: "インポート" }));
  await waitFor(() => expect(screen.getByTestId("import-dialog-close-btn")).toBeDisabled());

  await user.click(overlayOf(dialog));

  expect(closed()).toBe(false);
});

// ---- 子のダイアログ ----
// 子の背景・中のクリックは、子だけに効き、親には届かない。

test("FolderDialog から開いた登録解除の確認は、背景のクリックで確認だけが閉じ、中のクリックでは閉じない", async () => {
  const { user, dialog, closed, parentClosed } = await openFolderDeleteConfirmWithParent();

  await user.click(dialog.querySelector("p")!);
  await user.click(dialog);
  expect(closed()).toBe(false);
  expect(parentClosed()).toBe(false);

  await user.click(overlayOf(dialog));
  expect(closed()).toBe(true);
  expect(parentClosed()).toBe(false);
  expect(screen.getByTestId("folder-dialog")).toBeInTheDocument();
});

test("TagBarEditDialog から開いた削除の確認は、背景のクリックで確認だけが閉じ、中のクリックでは閉じない", async () => {
  const { user, closed } = await openTagBarEdit();
  await user.click(screen.getByTestId("tag-bar-edit-delete-btn"));
  const confirm = screen.getByTestId("confirm-dialog");
  // 子の初期フォーカスはキャンセルボタン。
  expect(document.activeElement).toBe(screen.getByTestId("confirm-dialog-cancel-btn"));

  await user.click(within(confirm).getByRole("heading"));
  await user.click(confirm);
  expect(screen.getByTestId("confirm-dialog")).toBeInTheDocument();
  expect(closed()).toBe(false);

  await user.click(overlayOf(confirm));
  expect(screen.queryByTestId("confirm-dialog")).toBeNull();
  expect(closed()).toBe(false);
  expect(screen.getByTestId("tag-bar-edit-dialog")).toBeInTheDocument();
  expect(api.deleteTag).not.toHaveBeenCalled();
});

test("WbImportDialog から開いた UnmatchedThumbnailsDialog は、背景のクリックで子だけが閉じ、中のクリックでは閉じない", async () => {
  vi.mocked(api.listUnmatchedThumbnails).mockResolvedValue([UNMATCHED_ROW]);
  const { user, container, closed } = await openWbImport();
  const openButton = await screen.findByTestId("import-unmatched-open-btn");
  await user.click(openButton);
  const child = await screen.findByTestId("unmatched-row");
  const childDialog = screen.getByTestId("unmatched-dialog");
  // 子はフォーカスを移さない(押した「確認する」ボタンのまま)。描画先は描画の場所の表に従う。
  expect(document.activeElement).toBe(openButton);
  const childPlacement = PLACEMENTS.find((p) => p.name === "UnmatchedThumbnailsDialog")!.placement;
  expect(overlayOf(childDialog).parentElement).toBe(
    childPlacement === "body" ? document.body : container,
  );
  expect(child).toBeInTheDocument();

  await user.click(within(childDialog).getByRole("heading"));
  await user.click(childDialog);
  expect(screen.getByTestId("unmatched-dialog")).toBeInTheDocument();
  expect(closed()).toBe(false);

  await user.click(overlayOf(childDialog));
  expect(screen.queryByTestId("unmatched-dialog")).toBeNull();
  expect(closed()).toBe(false);
  expect(screen.getByTestId("import-dialog")).toBeInTheDocument();
});

// 背景はどれも同じ z-index なので、重なりは DOM の順で決まる。子が親の上に出るには、
// 子の背景が親の背景より後ろにある必要がある。
test.each([
  {
    pair: "FolderDialog → 登録解除の確認",
    openChild: async () => (await openFolderDeleteConfirmWithParent()).dialog,
    parentTestId: "folder-dialog",
  },
  {
    pair: "TagBarEditDialog → 削除の確認",
    openChild: async () => {
      const { user } = await openTagBarEdit();
      await user.click(screen.getByTestId("tag-bar-edit-delete-btn"));
      return screen.getByTestId("confirm-dialog");
    },
    parentTestId: "tag-bar-edit-dialog",
  },
  {
    pair: "WbImportDialog → UnmatchedThumbnailsDialog",
    openChild: async () => {
      vi.mocked(api.listUnmatchedThumbnails).mockResolvedValue([UNMATCHED_ROW]);
      const { user } = await openWbImport();
      await user.click(await screen.findByTestId("import-unmatched-open-btn"));
      await screen.findByTestId("unmatched-row");
      return screen.getByTestId("unmatched-dialog");
    },
    parentTestId: "import-dialog",
  },
])("$pair の子の背景は、親の背景より DOM の後ろに描かれる", async ({ openChild, parentTestId }) => {
  const childOverlay = overlayOf(await openChild());
  const parentOverlay = overlayOf(screen.getByTestId(parentTestId));

  expect(
    parentOverlay.compareDocumentPosition(childOverlay) & Node.DOCUMENT_POSITION_FOLLOWING,
  ).toBeTruthy();
});
