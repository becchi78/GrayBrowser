// FolderDialog の現状の動作を固定する特性テスト。
import { beforeEach, expect, test, vi } from "vitest";
import { act, screen, waitFor, within } from "@testing-library/react";
import * as api from "../api";
import { UnreachableFoldersContext } from "../reachabilityContext";
import { renderWithProviders } from "../testing/render";
import type { RenameWatchFolderResult, ScanSummary } from "../types";
import { FolderDialog } from "./FolderDialog";

vi.mock("../api", () => ({
  countVideosUnderFolder: vi.fn(),
  listWatchCheckDisabledFolders: vi.fn(),
  listWatchFolders: vi.fn(),
  pickWatchFolders: vi.fn(),
  removeWatchFolder: vi.fn(),
  renameWatchFolder: vi.fn(),
  reorderWatchFolders: vi.fn(),
  setWatchFolderCheckEnabled: vi.fn(),
  startScan: vi.fn(),
}));

/** 外から解決・失敗させられる Promise。処理中の状態を作るために使う。 */
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

/** 解決しない Promise。取得が終わらない状態を保つために使う。 */
function never<T>(): Promise<T> {
  return new Promise<T>(() => {});
}

const SUMMARY: ScanSummary = {
  scanned: 5,
  registered: 2,
  reconciled: 1,
  unchanged: 1,
  skipped: 1,
  went_offline: 0,
  reactivated: 0,
  collisions: 0,
  unreachable_folders: [],
};
const SUMMARY_TEXT =
  "検出: 5 / 登録: 2 / 更新: 1 / 変化なし: 1 / スキップ: 1 / オフライン化: 0 / 復帰: 0";

beforeEach(() => {
  // 失敗の経路では console.error に出すだけなので、テストの出力を汚さないよう黙らせる。
  vi.spyOn(console, "error").mockImplementation(() => {});
  vi.mocked(api.listWatchFolders).mockResolvedValue(["A", "B", "C"]);
  vi.mocked(api.listWatchCheckDisabledFolders).mockResolvedValue([]);
});

function dialog(open: boolean, onClose: () => void, onChanged: () => void) {
  return <FolderDialog open={open} onClose={onClose} onChanged={onChanged} />;
}

/** 開いて、2つの一覧の取得が反映されるまで待つ。 */
async function renderOpen({ expectRows = 3 }: { expectRows?: number } = {}) {
  const onClose = vi.fn();
  const onChanged = vi.fn();
  const view = renderWithProviders(dialog(true, onClose, onChanged));
  await waitFor(() => expect(api.listWatchCheckDisabledFolders).toHaveBeenCalled());
  if (expectRows > 0) {
    await waitFor(() => expect(rows()).toHaveLength(expectRows));
  }
  // 取得の結果を落ち着かせる。
  await act(async () => {});
  // rerender は根の要素を差し替えるので、初回描画と同じ Provider で包まないと
  // 作り直されて状態が消え、閉じても状態が残ることを前提にした確認ができない。
  const setOpen = (open: boolean) =>
    view.rerender(
      <UnreachableFoldersContext.Provider value={[]}>
        {dialog(open, onClose, onChanged)}
      </UnreachableFoldersContext.Provider>,
    );
  return { ...view, onClose, onChanged, setOpen };
}

const rows = () => screen.queryAllByTestId("folder-row");
const paths = () => screen.queryAllByTestId("folder-row-path").map((el) => el.textContent);
const row = (path: string) => {
  const found = rows().find(
    (r) => within(r).queryByTestId("folder-row-path")?.textContent === path,
  );
  if (!found) {
    throw new Error(`行が見つからない: ${path}`);
  }
  return found;
};
const inRow = (path: string, testId: string) => within(row(path)).getByTestId(testId);
const checkbox = (path: string) => inRow(path, "folder-row-check-toggle") as HTMLInputElement;
const addButton = () => screen.getByTestId("folder-dialog-add-btn");
const scanButton = () => screen.getByTestId("folder-dialog-scan-btn");
const orderCheckError = () => screen.queryByTestId("folder-dialog-order-check-error");
const renameSummary = () => screen.queryByTestId("folder-dialog-rename-summary");
const confirmDialog = () => screen.queryByTestId("confirm-dialog");

// ---- 開閉と一覧の取得 ----

test("閉じているときは何も描画しない", () => {
  renderWithProviders(dialog(false, vi.fn(), vi.fn()));

  expect(screen.queryByTestId("folder-dialog")).toBeNull();
  expect(api.listWatchFolders).not.toHaveBeenCalled();
  expect(api.listWatchCheckDisabledFolders).not.toHaveBeenCalled();
});

test("開くと一覧を取得して表示し、チェックを外したフォルダのチェックは外れている", async () => {
  vi.mocked(api.listWatchCheckDisabledFolders).mockResolvedValue(["B"]);
  await renderOpen();

  expect(paths()).toEqual(["A", "B", "C"]);
  expect(checkbox("A").checked).toBe(true);
  expect(checkbox("B").checked).toBe(false);
  expect(checkbox("C").checked).toBe(true);
});

test("フォルダが無いときは、空の案内を出す", async () => {
  vi.mocked(api.listWatchFolders).mockResolvedValue([]);
  await renderOpen({ expectRows: 0 });

  expect(screen.getByText("登録済みのフォルダはありません。")).toBeInTheDocument();
});

test("チェックを外したフォルダの取得が失敗しても、一覧は表示する", async () => {
  vi.mocked(api.listWatchCheckDisabledFolders).mockRejectedValue(new Error("boom"));
  await renderOpen();

  expect(paths()).toEqual(["A", "B", "C"]);
  expect(checkbox("A").checked).toBe(true);
});

test("一覧の取得が失敗しても、チェックを外したフォルダの取得は反映する", async () => {
  vi.mocked(api.listWatchFolders).mockRejectedValue(new Error("boom"));
  // 開いたときの取得だけ成功させ、追加のあとの取得し直しは失敗させる。
  // 表示されるチェック状態は、開いたときに取得したものになる。
  vi.mocked(api.listWatchCheckDisabledFolders)
    .mockResolvedValueOnce(["B"])
    .mockRejectedValueOnce(new Error("refetch failed"));
  vi.mocked(api.pickWatchFolders).mockResolvedValue(["A", "B"]);
  const { user } = await renderOpen({ expectRows: 0 });

  await user.click(addButton());

  await waitFor(() => expect(paths()).toEqual(["A", "B"]));
  expect(checkbox("A").checked).toBe(true);
  expect(checkbox("B").checked).toBe(false);
});

test("閉じたあとに届いた一覧は捨て、開き直したときに反映しない", async () => {
  const first = deferred<string[]>();
  vi.mocked(api.listWatchFolders).mockReturnValueOnce(first.promise);
  const { setOpen } = await renderOpen({ expectRows: 0 });

  setOpen(false);
  await act(async () => {
    first.resolve(["STALE"]);
  });
  vi.mocked(api.listWatchFolders).mockReturnValueOnce(never());
  setOpen(true);
  await act(async () => {});

  expect(screen.getByTestId("folder-dialog")).toBeInTheDocument();
  expect(paths()).toEqual([]);
  expect(screen.getByText("登録済みのフォルダはありません。")).toBeInTheDocument();
});

test("閉じたあとに届いたチェック状態は捨て、開き直したときに反映しない", async () => {
  const first = deferred<string[]>();
  vi.mocked(api.listWatchCheckDisabledFolders).mockReturnValueOnce(first.promise);
  const { setOpen } = await renderOpen();

  setOpen(false);
  await act(async () => {
    first.resolve(["A"]);
  });
  vi.mocked(api.listWatchCheckDisabledFolders).mockReturnValueOnce(never());
  setOpen(true);
  await waitFor(() => expect(rows()).toHaveLength(3));
  await act(async () => {});

  expect(checkbox("A").checked).toBe(true);
});

test("開き直したときは、前回のエラー表示を消す", async () => {
  vi.mocked(api.reorderWatchFolders).mockRejectedValue("rejected order");
  const { user, setOpen } = await renderOpen();

  await user.click(inRow("A", "folder-row-move-down-btn"));
  await waitFor(() => expect(orderCheckError()).toHaveTextContent("rejected order"));

  setOpen(false);
  setOpen(true);

  await waitFor(() => expect(orderCheckError()).toBeNull());
  expect(paths()).toEqual(["A", "B", "C"]);
});

// 今の動作を固めている: エラーを消すのは、開き直したときの一覧の取得が
// 成功したときだけ。望ましい動作と決めたものではなく、変えるときは意図して更新する。
test("現状: 開き直したときに一覧の取得が失敗すると、前回のエラー表示が残る", async () => {
  vi.mocked(api.reorderWatchFolders).mockRejectedValue("rejected order");
  const { user, setOpen } = await renderOpen();

  await user.click(inRow("A", "folder-row-move-down-btn"));
  await waitFor(() => expect(orderCheckError()).toHaveTextContent("rejected order"));

  vi.mocked(api.listWatchFolders).mockRejectedValueOnce(new Error("boom"));
  const fetchesBefore = vi.mocked(api.listWatchFolders).mock.calls.length;
  setOpen(false);
  setOpen(true);
  await waitFor(() =>
    expect(api.listWatchFolders).toHaveBeenCalledTimes(fetchesBefore + 1),
  );
  await act(async () => {});

  expect(orderCheckError()).toHaveTextContent("rejected order");
  expect(paths()).toEqual(["A", "B", "C"]);
});

test("背景のクリックと「閉じる」で onClose を呼び、ダイアログの中のクリックでは呼ばない", async () => {
  const { user, onClose } = await renderOpen();

  await user.click(screen.getByTestId("folder-dialog"));
  expect(onClose).not.toHaveBeenCalled();

  await user.click(screen.getByTestId("folder-dialog").parentElement!);
  expect(onClose).toHaveBeenCalledTimes(1);

  await user.click(screen.getByTestId("folder-dialog-close-btn"));
  expect(onClose).toHaveBeenCalledTimes(2);
});

// ---- 並べ替え ----

test("端の行では、↑ または ↓ を押せない", async () => {
  await renderOpen();

  expect(inRow("A", "folder-row-move-up-btn")).toBeDisabled();
  expect(inRow("A", "folder-row-move-down-btn")).toBeEnabled();
  expect(inRow("B", "folder-row-move-up-btn")).toBeEnabled();
  expect(inRow("B", "folder-row-move-down-btn")).toBeEnabled();
  expect(inRow("C", "folder-row-move-up-btn")).toBeEnabled();
  expect(inRow("C", "folder-row-move-down-btn")).toBeDisabled();
});

test("並べ替えは、バックエンドが返した順序で表示を置き換え、onChanged を呼ぶ", async () => {
  // 送った順序とは違う順序を返し、表示が送った順序ではなく返した順序になることを見る。
  vi.mocked(api.reorderWatchFolders).mockResolvedValue(["C", "B", "A"]);
  const { user, onChanged } = await renderOpen();

  await user.click(inRow("A", "folder-row-move-down-btn"));

  expect(api.reorderWatchFolders).toHaveBeenCalledWith(["B", "A", "C"]);
  await waitFor(() => expect(paths()).toEqual(["C", "B", "A"]));
  expect(onChanged).toHaveBeenCalledTimes(1);
});

test("↑ は1つ上と入れ替えた順序を保存する", async () => {
  vi.mocked(api.reorderWatchFolders).mockImplementation(async (next) => next);
  const { user } = await renderOpen();

  await user.click(inRow("C", "folder-row-move-up-btn"));

  expect(api.reorderWatchFolders).toHaveBeenCalledWith(["A", "C", "B"]);
  await waitFor(() => expect(paths()).toEqual(["A", "C", "B"]));
});

test("続けて並べ替えると、直前に保存された順序から計算する", async () => {
  vi.mocked(api.reorderWatchFolders).mockImplementation(async (next) => next);
  const { user } = await renderOpen();

  await user.click(inRow("A", "folder-row-move-down-btn"));
  await waitFor(() => expect(paths()).toEqual(["B", "A", "C"]));
  await user.click(inRow("A", "folder-row-move-down-btn"));

  expect(api.reorderWatchFolders).toHaveBeenLastCalledWith(["B", "C", "A"]);
  await waitFor(() => expect(paths()).toEqual(["B", "C", "A"]));
});

test("保存が拒否されたら表示は変えずにエラーを出し、onChanged は呼ばない", async () => {
  vi.mocked(api.reorderWatchFolders).mockRejectedValue("list changed");
  const { user, onChanged } = await renderOpen();

  await user.click(inRow("A", "folder-row-move-down-btn"));

  await waitFor(() => expect(orderCheckError()).toHaveTextContent("list changed"));
  expect(paths()).toEqual(["A", "B", "C"]);
  expect(onChanged).not.toHaveBeenCalled();
  // 失敗のあとは、また押せる。
  expect(inRow("A", "folder-row-move-down-btn")).toBeEnabled();
});

test("次の並べ替えを始めると、前のエラー表示を消す", async () => {
  vi.mocked(api.reorderWatchFolders)
    .mockRejectedValueOnce("list changed")
    .mockReturnValueOnce(never());
  const { user } = await renderOpen();

  await user.click(inRow("A", "folder-row-move-down-btn"));
  await waitFor(() => expect(orderCheckError()).not.toBeNull());

  await user.click(inRow("A", "folder-row-move-down-btn"));

  expect(orderCheckError()).toBeNull();
});

test("並べ替えの保存中は、すべての ↑/↓ と追加を押せない", async () => {
  const save = deferred<string[]>();
  vi.mocked(api.reorderWatchFolders).mockReturnValue(save.promise);
  const { user } = await renderOpen();

  await user.click(inRow("B", "folder-row-move-down-btn"));

  for (const path of ["A", "B", "C"]) {
    expect(inRow(path, "folder-row-move-up-btn")).toBeDisabled();
    expect(inRow(path, "folder-row-move-down-btn")).toBeDisabled();
  }
  expect(addButton()).toBeDisabled();
  expect(api.reorderWatchFolders).toHaveBeenCalledTimes(1);

  await act(async () => {
    save.resolve(["A", "C", "B"]);
  });

  expect(paths()).toEqual(["A", "C", "B"]);
  expect(inRow("A", "folder-row-move-down-btn")).toBeEnabled();
  expect(inRow("C", "folder-row-move-up-btn")).toBeEnabled();
  expect(addButton()).toBeEnabled();
});

// ---- 更新チェックの切り替え ----

test("更新チェックを切り替えると、返ってきた状態を表示し、onChanged は呼ばない", async () => {
  vi.mocked(api.setWatchFolderCheckEnabled).mockResolvedValue(["A"]);
  const { user, onChanged } = await renderOpen();

  await user.click(checkbox("A"));

  expect(api.setWatchFolderCheckEnabled).toHaveBeenCalledWith("A", false);
  await waitFor(() => expect(checkbox("A").checked).toBe(false));
  expect(onChanged).not.toHaveBeenCalled();
});

test("外れているチェックを入れると、有効にする要求を送る", async () => {
  vi.mocked(api.listWatchCheckDisabledFolders).mockResolvedValue(["B"]);
  vi.mocked(api.setWatchFolderCheckEnabled).mockResolvedValue([]);
  const { user } = await renderOpen();

  await user.click(checkbox("B"));

  expect(api.setWatchFolderCheckEnabled).toHaveBeenCalledWith("B", true);
  await waitFor(() => expect(checkbox("B").checked).toBe(true));
});

test("切り替え中は、すべての行のチェックと追加を押せない", async () => {
  const toggle = deferred<string[]>();
  vi.mocked(api.setWatchFolderCheckEnabled).mockReturnValue(toggle.promise);
  const { user, onChanged } = await renderOpen();

  await user.click(checkbox("B"));

  for (const path of ["A", "B", "C"]) {
    expect(checkbox(path)).toBeDisabled();
  }
  expect(addButton()).toBeDisabled();

  await act(async () => {
    toggle.resolve(["B"]);
  });

  for (const path of ["A", "B", "C"]) {
    expect(checkbox(path)).toBeEnabled();
  }
  expect(addButton()).toBeEnabled();
  expect(checkbox("B").checked).toBe(false);
  expect(onChanged).not.toHaveBeenCalled();
});

test("切り替えが失敗したら、チェックは変えずにエラーを出す", async () => {
  vi.mocked(api.setWatchFolderCheckEnabled).mockRejectedValue("toggle failed");
  const { user, onChanged } = await renderOpen();

  await user.click(checkbox("A"));

  await waitFor(() => expect(orderCheckError()).toHaveTextContent("toggle failed"));
  expect(checkbox("A").checked).toBe(true);
  expect(checkbox("A")).toBeEnabled();
  expect(onChanged).not.toHaveBeenCalled();
});

test("次の切り替えを始めると、前のエラー表示を消す", async () => {
  vi.mocked(api.setWatchFolderCheckEnabled)
    .mockRejectedValueOnce("toggle failed")
    .mockReturnValueOnce(never());
  const { user } = await renderOpen();

  await user.click(checkbox("A"));
  await waitFor(() => expect(orderCheckError()).not.toBeNull());

  await user.click(checkbox("A"));

  expect(orderCheckError()).toBeNull();
});

// ---- 追加 ----

test("追加すると、返ってきた一覧で置き換え、onChanged を呼び、チェック状態を取得し直す", async () => {
  vi.mocked(api.pickWatchFolders).mockResolvedValue(["A", "B", "C", "D"]);
  const { user, onChanged } = await renderOpen();
  vi.mocked(api.listWatchCheckDisabledFolders).mockResolvedValue(["D"]);
  const fetchesBefore = vi.mocked(api.listWatchCheckDisabledFolders).mock.calls.length;

  await user.click(addButton());

  await waitFor(() => expect(paths()).toEqual(["A", "B", "C", "D"]));
  expect(onChanged).toHaveBeenCalledTimes(1);
  expect(api.listWatchCheckDisabledFolders).toHaveBeenCalledTimes(fetchesBefore + 1);
  await waitFor(() => expect(checkbox("D").checked).toBe(false));
});

test("追加のあとのチェック状態の取得し直しが失敗しても、一覧と onChanged は反映する", async () => {
  vi.mocked(api.pickWatchFolders).mockResolvedValue(["A", "B", "C", "D"]);
  const { user, onChanged } = await renderOpen();
  vi.mocked(api.listWatchCheckDisabledFolders).mockRejectedValue(new Error("boom"));

  await user.click(addButton());

  await waitFor(() => expect(paths()).toEqual(["A", "B", "C", "D"]));
  expect(onChanged).toHaveBeenCalledTimes(1);
});

test("追加が失敗したら、一覧は変えず、onChanged も呼ばない", async () => {
  vi.mocked(api.pickWatchFolders).mockRejectedValue(new Error("cancelled"));
  const { user, onChanged } = await renderOpen();

  await user.click(addButton());
  await act(async () => {});

  expect(paths()).toEqual(["A", "B", "C"]);
  expect(onChanged).not.toHaveBeenCalled();
});

// ---- 再スキャン ----

test("フォルダが無いときは再スキャンを押せず、理由を出す", async () => {
  vi.mocked(api.listWatchFolders).mockResolvedValue([]);
  await renderOpen({ expectRows: 0 });

  expect(scanButton()).toBeDisabled();
  expect(screen.getByTestId("folder-dialog-scan-note")).toHaveTextContent(
    "フォルダが登録されていないため、再スキャンできません。",
  );
});

test("すべてのチェックが外れているときは再スキャンを押せず、理由を出す", async () => {
  vi.mocked(api.listWatchCheckDisabledFolders).mockResolvedValue(["A", "B", "C"]);
  await renderOpen();

  expect(scanButton()).toBeDisabled();
  expect(screen.getByTestId("folder-dialog-scan-note")).toHaveTextContent(
    "すべてのフォルダで「更新チェック」が外れているため、再スキャンできません。",
  );
});

test("対象があるときは再スキャンを押せ、対象の説明を出す", async () => {
  vi.mocked(api.listWatchCheckDisabledFolders).mockResolvedValue(["A"]);
  await renderOpen();

  expect(scanButton()).toBeEnabled();
  expect(scanButton()).toHaveTextContent("再スキャン");
  expect(screen.getByTestId("folder-dialog-scan-note")).toHaveTextContent(
    "「更新チェック」が入っているフォルダを、あらためて全件確認します。チェックを外したフォルダは対象外です。",
  );
});

test("スキャン中は押せず、終わったら要約を出して onChanged を呼ぶ", async () => {
  const scan = deferred<ScanSummary>();
  vi.mocked(api.startScan).mockReturnValue(scan.promise);
  const { user, onChanged } = await renderOpen();

  await user.click(scanButton());

  expect(scanButton()).toBeDisabled();
  expect(scanButton()).toHaveTextContent("スキャン中...");
  expect(onChanged).not.toHaveBeenCalled();

  await act(async () => {
    scan.resolve(SUMMARY);
  });

  expect(scanButton()).toBeEnabled();
  expect(scanButton()).toHaveTextContent("再スキャン");
  expect(screen.getByText(SUMMARY_TEXT)).toHaveClass("scan-summary");
  expect(onChanged).toHaveBeenCalledTimes(1);
});

test("次のスキャンを始めると、前の要約を消す", async () => {
  vi.mocked(api.startScan).mockResolvedValueOnce(SUMMARY).mockReturnValueOnce(never());
  const { user } = await renderOpen();

  await user.click(scanButton());
  await waitFor(() => expect(screen.getByText(SUMMARY_TEXT)).toBeInTheDocument());

  await user.click(scanButton());

  expect(screen.queryByText(SUMMARY_TEXT)).toBeNull();
});

test("スキャンが失敗したら、また押せるようになり、onChanged は呼ばない", async () => {
  vi.mocked(api.startScan).mockRejectedValue(new Error("scan failed"));
  const { user, onChanged } = await renderOpen();

  await user.click(scanButton());

  await waitFor(() => expect(scanButton()).toBeEnabled());
  expect(screen.queryByText(SUMMARY_TEXT)).toBeNull();
  expect(onChanged).not.toHaveBeenCalled();
});

// ---- パスの変更 ----

test("編集を始めると、入力欄に今のパスが入り、ほかの行は通常の表示のまま", async () => {
  const { user } = await renderOpen();

  await user.click(inRow("B", "folder-row-edit-btn"));

  const input = screen.getByTestId("folder-row-path-input") as HTMLInputElement;
  expect(input.value).toBe("B");
  expect(within(row("A")).queryByTestId("folder-row-path-input")).toBeNull();
  expect(screen.getAllByTestId("folder-row-path-input")).toHaveLength(1);
});

test("キャンセルすると、編集をやめて元の表示に戻る", async () => {
  const { user } = await renderOpen();

  await user.click(inRow("B", "folder-row-edit-btn"));
  await user.click(screen.getByTestId("folder-row-cancel-btn"));

  expect(screen.queryByTestId("folder-row-path-input")).toBeNull();
  expect(paths()).toEqual(["A", "B", "C"]);
  expect(api.renameWatchFolder).not.toHaveBeenCalled();
});

test("空のパスは「パスを入力してください」で止める", async () => {
  const { user, onChanged } = await renderOpen();

  await user.click(inRow("B", "folder-row-edit-btn"));
  await user.clear(screen.getByTestId("folder-row-path-input"));
  await user.type(screen.getByTestId("folder-row-path-input"), "   ");
  await user.click(screen.getByTestId("folder-row-save-btn"));

  expect(screen.getByTestId("folder-row-edit-error")).toHaveTextContent("パスを入力してください");
  expect(api.renameWatchFolder).not.toHaveBeenCalled();
  expect(onChanged).not.toHaveBeenCalled();
});

test("変更に成功したら、件数を出し、一覧を置き換え、onChanged を呼び、チェック状態を取得し直す", async () => {
  vi.mocked(api.renameWatchFolder).mockResolvedValue({
    folders: ["A", "B2", "C"],
    renamed_count: 3,
    collision_skipped_count: 0,
  });
  const { user, onChanged } = await renderOpen();
  const fetchesBefore = vi.mocked(api.listWatchCheckDisabledFolders).mock.calls.length;

  await user.click(inRow("B", "folder-row-edit-btn"));
  await user.clear(screen.getByTestId("folder-row-path-input"));
  await user.type(screen.getByTestId("folder-row-path-input"), "  B2  ");
  await user.click(screen.getByTestId("folder-row-save-btn"));

  expect(api.renameWatchFolder).toHaveBeenCalledWith("B", "B2");
  await waitFor(() => expect(renameSummary()).toHaveTextContent("3件のパスを更新しました"));
  expect(renameSummary()).toHaveTextContent(/^3件のパスを更新しました$/);
  expect(paths()).toEqual(["A", "B2", "C"]);
  expect(screen.queryByTestId("folder-row-path-input")).toBeNull();
  expect(onChanged).toHaveBeenCalledTimes(1);
  await waitFor(() =>
    expect(api.listWatchCheckDisabledFolders).toHaveBeenCalledTimes(fetchesBefore + 1),
  );
});

test("衝突があれば、その件数も出す", async () => {
  vi.mocked(api.renameWatchFolder).mockResolvedValue({
    folders: ["A", "B2", "C"],
    renamed_count: 3,
    collision_skipped_count: 2,
  });
  const { user } = await renderOpen();

  await user.click(inRow("B", "folder-row-edit-btn"));
  await user.clear(screen.getByTestId("folder-row-path-input"));
  await user.type(screen.getByTestId("folder-row-path-input"), "B2");
  await user.click(screen.getByTestId("folder-row-save-btn"));

  await waitFor(() =>
    expect(renameSummary()).toHaveTextContent(
      "3件のパスを更新しました（2件は既存の登録と衝突したためスキップされました）",
    ),
  );
});

test("変更のあとのチェック状態の取得し直しが失敗しても、件数と onChanged は反映する", async () => {
  vi.mocked(api.renameWatchFolder).mockResolvedValue({
    folders: ["A", "B2", "C"],
    renamed_count: 1,
    collision_skipped_count: 0,
  });
  const { user, onChanged } = await renderOpen();
  vi.mocked(api.listWatchCheckDisabledFolders).mockRejectedValue(new Error("boom"));

  await user.click(inRow("B", "folder-row-edit-btn"));
  await user.clear(screen.getByTestId("folder-row-path-input"));
  await user.type(screen.getByTestId("folder-row-path-input"), "B2");
  await user.click(screen.getByTestId("folder-row-save-btn"));

  await waitFor(() => expect(renameSummary()).toHaveTextContent("1件のパスを更新しました"));
  expect(paths()).toEqual(["A", "B2", "C"]);
  expect(onChanged).toHaveBeenCalledTimes(1);
});

test("変更が失敗したら、行の中にエラーを出し、編集を続ける", async () => {
  vi.mocked(api.renameWatchFolder).mockRejectedValue("duplicate path");
  const { user, onChanged } = await renderOpen();

  await user.click(inRow("B", "folder-row-edit-btn"));
  await user.clear(screen.getByTestId("folder-row-path-input"));
  await user.type(screen.getByTestId("folder-row-path-input"), "A");
  await user.click(screen.getByTestId("folder-row-save-btn"));

  // 編集中の行はパスの表示を持たないので、位置で取る。
  const error = await within(rows()[1]).findByTestId("folder-row-edit-error");
  expect(error).toHaveTextContent("duplicate path");
  expect(screen.getByTestId("folder-row-path-input")).toBeEnabled();
  expect(screen.getByTestId("folder-row-save-btn")).toBeEnabled();
  expect(renameSummary()).toBeNull();
  expect(onChanged).not.toHaveBeenCalled();
});

test("保存中は、入力とボタンを押せない", async () => {
  const save = deferred<RenameWatchFolderResult>();
  vi.mocked(api.renameWatchFolder).mockReturnValue(save.promise);
  const { user } = await renderOpen();

  await user.click(inRow("B", "folder-row-edit-btn"));
  await user.clear(screen.getByTestId("folder-row-path-input"));
  await user.type(screen.getByTestId("folder-row-path-input"), "B2");
  await user.click(screen.getByTestId("folder-row-save-btn"));

  expect(screen.getByTestId("folder-row-path-input")).toBeDisabled();
  expect(screen.getByTestId("folder-row-save-btn")).toBeDisabled();
  expect(screen.getByTestId("folder-row-save-btn")).toHaveTextContent("保存中...");
  expect(screen.getByTestId("folder-row-cancel-btn")).toBeDisabled();

  await act(async () => {
    save.resolve({ folders: ["A", "B2", "C"], renamed_count: 1, collision_skipped_count: 0 });
  });

  expect(screen.queryByTestId("folder-row-path-input")).toBeNull();
  expect(paths()).toEqual(["A", "B2", "C"]);
});

test("編集を始めると、前回の変更結果の表示を消す", async () => {
  vi.mocked(api.renameWatchFolder).mockResolvedValue({
    folders: ["A", "B2", "C"],
    renamed_count: 1,
    collision_skipped_count: 0,
  });
  const { user } = await renderOpen();

  await user.click(inRow("B", "folder-row-edit-btn"));
  await user.clear(screen.getByTestId("folder-row-path-input"));
  await user.type(screen.getByTestId("folder-row-path-input"), "B2");
  await user.click(screen.getByTestId("folder-row-save-btn"));
  await waitFor(() => expect(renameSummary()).not.toBeNull());

  await user.click(inRow("A", "folder-row-edit-btn"));

  expect(renameSummary()).toBeNull();
});

test("別の行で編集を始めると、前の行のエラーを消す", async () => {
  const { user } = await renderOpen();

  await user.click(inRow("B", "folder-row-edit-btn"));
  await user.clear(screen.getByTestId("folder-row-path-input"));
  await user.click(screen.getByTestId("folder-row-save-btn"));
  expect(screen.getByTestId("folder-row-edit-error")).toHaveTextContent("パスを入力してください");

  await user.click(inRow("A", "folder-row-edit-btn"));

  expect(screen.queryByTestId("folder-row-edit-error")).toBeNull();
  const inputs = screen.getAllByTestId("folder-row-path-input") as HTMLInputElement[];
  expect(inputs).toHaveLength(1);
  expect(inputs[0].value).toBe("A");
});

// ---- 登録解除 ----

test("登録解除は、件数を取得してから確認を出す", async () => {
  const count = deferred<number>();
  vi.mocked(api.countVideosUnderFolder).mockReturnValue(count.promise);
  const { user } = await renderOpen();

  await user.click(inRow("B", "folder-row-delete-btn"));

  expect(api.countVideosUnderFolder).toHaveBeenCalledWith("B");
  expect(confirmDialog()).toBeNull();

  await act(async () => {
    count.resolve(42);
  });

  const confirm = confirmDialog()!;
  expect(confirm).toBeInTheDocument();
  expect(within(confirm).getByRole("heading")).toHaveTextContent("フォルダの登録を解除");
  expect(screen.getByTestId("confirm-dialog-confirm-btn")).toHaveTextContent("登録を解除する");
  expect(screen.getByTestId("confirm-dialog-confirm-btn")).toHaveClass(
    "confirm-dialog-confirm--danger",
  );
  expect(document.activeElement).toBe(screen.getByTestId("confirm-dialog-cancel-btn"));
  expect(within(confirm).getByText("42").tagName).toBe("STRONG");
  expect(confirm.querySelector(".folder-delete-confirm-path")).toHaveTextContent(/^B$/);
  expect(api.removeWatchFolder).not.toHaveBeenCalled();
});

test("件数の取得が失敗したら、確認を出さない", async () => {
  vi.mocked(api.countVideosUnderFolder).mockRejectedValue(new Error("boom"));
  const { user } = await renderOpen();

  await user.click(inRow("B", "folder-row-delete-btn"));
  await act(async () => {});

  expect(confirmDialog()).toBeNull();
});

test("確認でキャンセルすると、解除せずに閉じる", async () => {
  vi.mocked(api.countVideosUnderFolder).mockResolvedValue(1);
  const { user, onChanged, onClose } = await renderOpen();

  await user.click(inRow("B", "folder-row-delete-btn"));
  await screen.findByTestId("confirm-dialog");
  await user.click(screen.getByTestId("confirm-dialog-cancel-btn"));

  expect(confirmDialog()).toBeNull();
  expect(api.removeWatchFolder).not.toHaveBeenCalled();
  expect(paths()).toEqual(["A", "B", "C"]);
  expect(onChanged).not.toHaveBeenCalled();
  expect(onClose).not.toHaveBeenCalled();
});

test("解除中でなければ、確認の背景のクリックで確認を閉じ、確認の中のクリックでは閉じない", async () => {
  vi.mocked(api.countVideosUnderFolder).mockResolvedValue(1);
  const { user, onClose } = await renderOpen();

  await user.click(inRow("B", "folder-row-delete-btn"));
  const confirm = await screen.findByTestId("confirm-dialog");

  await user.click(confirm);
  expect(confirmDialog()).toBeInTheDocument();

  await user.click(confirm.parentElement!);
  expect(confirmDialog()).toBeNull();
  expect(api.removeWatchFolder).not.toHaveBeenCalled();
  expect(onClose).not.toHaveBeenCalled();
});

test("解除中は、ボタンを押せず、背景をクリックしても閉じない。終わったら一覧を置き換えて onChanged を呼ぶ", async () => {
  vi.mocked(api.countVideosUnderFolder).mockResolvedValue(1);
  const removal = deferred<string[]>();
  vi.mocked(api.removeWatchFolder).mockReturnValue(removal.promise);
  const { user, onChanged } = await renderOpen();
  vi.mocked(api.listWatchCheckDisabledFolders).mockResolvedValue(["C"]);
  const fetchesBefore = vi.mocked(api.listWatchCheckDisabledFolders).mock.calls.length;

  await user.click(inRow("B", "folder-row-delete-btn"));
  const confirm = await screen.findByTestId("confirm-dialog");
  await user.click(screen.getByTestId("confirm-dialog-confirm-btn"));

  expect(api.removeWatchFolder).toHaveBeenCalledWith("B");
  expect(screen.getByTestId("confirm-dialog-confirm-btn")).toBeDisabled();
  expect(screen.getByTestId("confirm-dialog-confirm-btn")).toHaveTextContent("処理中...");
  expect(screen.getByTestId("confirm-dialog-cancel-btn")).toBeDisabled();

  await user.click(confirm.parentElement!);
  expect(confirmDialog()).toBeInTheDocument();
  expect(onChanged).not.toHaveBeenCalled();

  await act(async () => {
    removal.resolve(["A", "C"]);
  });

  expect(confirmDialog()).toBeNull();
  expect(paths()).toEqual(["A", "C"]);
  expect(onChanged).toHaveBeenCalledTimes(1);
  await waitFor(() => expect(checkbox("C").checked).toBe(false));
  expect(api.listWatchCheckDisabledFolders).toHaveBeenCalledTimes(fetchesBefore + 1);
});

test("解除のあとのチェック状態の取得し直しが失敗しても、一覧と onChanged は反映する", async () => {
  vi.mocked(api.countVideosUnderFolder).mockResolvedValue(1);
  vi.mocked(api.removeWatchFolder).mockResolvedValue(["A", "C"]);
  const { user, onChanged } = await renderOpen();
  vi.mocked(api.listWatchCheckDisabledFolders).mockRejectedValue(new Error("boom"));

  await user.click(inRow("B", "folder-row-delete-btn"));
  await screen.findByTestId("confirm-dialog");
  await user.click(screen.getByTestId("confirm-dialog-confirm-btn"));

  await waitFor(() => expect(paths()).toEqual(["A", "C"]));
  expect(confirmDialog()).toBeNull();
  expect(onChanged).toHaveBeenCalledTimes(1);
});

test("解除が失敗したら、確認を開いたまま押せる状態に戻し、onChanged は呼ばない", async () => {
  vi.mocked(api.countVideosUnderFolder).mockResolvedValue(1);
  vi.mocked(api.removeWatchFolder).mockRejectedValue(new Error("remove failed"));
  const { user, onChanged } = await renderOpen();

  await user.click(inRow("B", "folder-row-delete-btn"));
  await screen.findByTestId("confirm-dialog");
  await user.click(screen.getByTestId("confirm-dialog-confirm-btn"));

  await waitFor(() => expect(screen.getByTestId("confirm-dialog-confirm-btn")).toBeEnabled());
  expect(confirmDialog()).toBeInTheDocument();
  expect(paths()).toEqual(["A", "B", "C"]);
  expect(onChanged).not.toHaveBeenCalled();
});
