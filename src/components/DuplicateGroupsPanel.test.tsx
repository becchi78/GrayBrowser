// DuplicateGroupsPanel の現状の動作を固定する特性テスト。
import { beforeEach, expect, test, vi } from "vitest";
import { act, screen, waitFor, within } from "@testing-library/react";
import * as api from "../api";
import * as events from "../events";
import { UnreachableFoldersContext } from "../reachabilityContext";
import { renderWithProviders } from "../testing/render";
import type { DuplicateGroup, DuplicateGroupKind, DuplicateGroupMember } from "../types";
import { DuplicateGroupsPanel } from "./DuplicateGroupsPanel";

vi.mock("../api", () => ({
  deleteDuplicateVideo: vi.fn(),
  deleteDuplicateVideoAndFile: vi.fn(),
  getThumbnails: vi.fn(),
  listDuplicateGroups: vi.fn(),
  playVideo: vi.fn(),
  refreshDuplicateGroups: vi.fn(),
}));

vi.mock("../events", () => ({
  onDedupUpdated: vi.fn(),
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

/** 解決しない Promise。処理が終わらない状態を保つために使う。 */
function never<T>(): Promise<T> {
  return new Promise<T>(() => {});
}

const pathOf = (id: string) => `C:\\videos\\${id}.mp4`;

function member(id: string, overrides: Partial<DuplicateGroupMember> = {}): DuplicateGroupMember {
  return {
    video_id: id,
    file_path: pathOf(id),
    file_name: `${id}.mp4`,
    file_size: 100 * 1024 * 1024,
    status: "online",
    created_at: "2024-01-01 00:00:00",
    duration: 60,
    width: 1920,
    height: 1080,
    ...overrides,
  };
}

function group(
  members: DuplicateGroupMember[],
  kind: DuplicateGroupKind = "quick_hash_confirmed",
): DuplicateGroup {
  return { kind, members };
}

/** どの行でも「ファイルを削除」が押せる、オンライン3件の確定グループ。 */
const THREE = () => [
  group([
    member("a", { created_at: "2024-01-01 00:00:00" }),
    member("b", { created_at: "2024-01-02 00:00:00" }),
    member("c", { created_at: "2024-01-03 00:00:00" }),
  ]),
];

// dedup:updated の購読に渡されたコールバックと、その解除関数。
let emitDedup: ((groups: DuplicateGroup[]) => void) | null = null;
let unlisten = vi.fn();

beforeEach(() => {
  // 失敗の経路では console.error に出すだけなので、テストの出力を汚さないよう黙らせる。
  vi.spyOn(console, "error").mockImplementation(() => {});
  emitDedup = null;
  unlisten = vi.fn();
  vi.mocked(events.onDedupUpdated).mockImplementation((cb) => {
    emitDedup = cb;
    return unlisten;
  });
  vi.mocked(api.getThumbnails).mockResolvedValue(null);
  vi.mocked(api.playVideo).mockResolvedValue(undefined);
});

/** 連なった Promise の解決と、それに続く再描画を落ち着かせる。 */
async function settle() {
  for (let i = 0; i < 3; i++) {
    await act(async () => {});
  }
}

async function emit(groups: DuplicateGroup[]) {
  await act(async () => {
    emitDedup!(groups);
  });
}

interface PanelProps {
  refreshKey: number;
  open: boolean;
}

async function renderPanel({
  groups = THREE(),
  open = true,
  unreachableFolders = [],
}: {
  groups?: DuplicateGroup[];
  open?: boolean;
  unreachableFolders?: readonly string[];
} = {}) {
  vi.mocked(api.listDuplicateGroups).mockResolvedValue(groups);
  const onCountChange = vi.fn();
  let current: PanelProps = { refreshKey: 0, open };
  const ui = (p: PanelProps) => (
    <DuplicateGroupsPanel refreshKey={p.refreshKey} open={p.open} onCountChange={onCountChange} />
  );
  const view = renderWithProviders(ui(current), { unreachableFolders });
  await waitFor(() => expect(api.listDuplicateGroups).toHaveBeenCalled());
  await settle();
  // rerender は根の要素を差し替えるので、初回描画と同じ Provider で包まないと
  // 作り直されて状態が消え、閉じても状態が残ることを前提にした確認ができない。
  const update = async (next: Partial<PanelProps>) => {
    current = { ...current, ...next };
    view.rerender(
      <UnreachableFoldersContext.Provider value={unreachableFolders}>
        {ui(current)}
      </UnreachableFoldersContext.Provider>,
    );
    await settle();
  };
  return { ...view, onCountChange, update };
}

const panel = () => screen.queryByTestId("status-panel");
const rows = () =>
  Array.from(document.querySelectorAll<HTMLElement>("li.duplicate-group-member"));
const paths = () =>
  rows().map((r) => r.querySelector(".duplicate-group-member-path")?.textContent);
const row = (id: string) => {
  const found = rows().find(
    (r) => r.querySelector(".duplicate-group-member-path")?.textContent === pathOf(id),
  );
  if (!found) {
    throw new Error(`行が見つからない: ${id}`);
  }
  return found;
};
const unregisterButton = (id: string) => within(row(id)).getByRole("button", { name: "登録解除" });
const fileDeleteButton = (id: string) =>
  within(row(id)).getByRole("button", { name: "ファイルを削除" });
const thumb = (id: string) => row(id).querySelector<HTMLElement>(".duplicate-group-member-thumb")!;
const notes = (id: string) =>
  Array.from(row(id).querySelectorAll(".duplicate-group-member-play-note")).map(
    (el) => el.textContent,
  );
const rowErrors = (id: string) =>
  Array.from(row(id).querySelectorAll(".duplicate-group-member-play-error")).map(
    (el) => el.textContent,
  );
const panelError = () => document.querySelector(".duplicate-groups-error");
const refreshButton = () =>
  within(panel()!).getByRole("button", { name: /重複を再チェック|確認中\.\.\./ });
const confirmDialog = () => screen.queryByTestId("confirm-dialog");
const confirmButton = () => screen.getByTestId("confirm-dialog-confirm-btn");
const cancelButton = () => screen.getByTestId("confirm-dialog-cancel-btn");
const allRowButtons = () =>
  rows().flatMap((r) => Array.from(r.querySelectorAll<HTMLButtonElement>("button")));

// ---- 一覧の取得と購読 ----

test("開くと一覧を取得し、グループとメンバーを表示する", async () => {
  await renderPanel();

  expect(panel()).toHaveClass("duplicate-groups-panel", "status-panel");
  expect(document.querySelectorAll("li.duplicate-group")).toHaveLength(1);
  expect(paths()).toEqual([pathOf("a"), pathOf("b"), pathOf("c")]);
  expect(api.listDuplicateGroups).toHaveBeenCalledTimes(1);
});

test("グループが無いときは、空の案内を出す", async () => {
  await renderPanel({ groups: [] });

  expect(screen.getByText("重複候補は見つかっていません。")).toBeInTheDocument();
  expect(document.querySelector(".duplicate-groups-list")).toBeNull();
});

test("一覧の取得が失敗しても、空の案内を出す", async () => {
  vi.mocked(api.listDuplicateGroups).mockRejectedValueOnce(new Error("boom"));
  await renderPanel();

  expect(screen.getByText("重複候補は見つかっていません。")).toBeInTheDocument();
});

test("refreshKey が変わるたびに取得し直し、変わらなければ取得しない", async () => {
  const { update } = await renderPanel();

  await update({ open: true });
  expect(api.listDuplicateGroups).toHaveBeenCalledTimes(1);

  vi.mocked(api.listDuplicateGroups).mockResolvedValue([group([member("x"), member("y")])]);
  await update({ refreshKey: 1 });

  expect(api.listDuplicateGroups).toHaveBeenCalledTimes(2);
  expect(paths()).toEqual([pathOf("x"), pathOf("y")]);
});

test("dedup:updated を受けたら、一覧を置き換える", async () => {
  await renderPanel();

  await emit([group([member("p"), member("q")], "path_collision_confirmed")]);

  expect(paths()).toEqual([pathOf("p"), pathOf("q")]);
});

test("購読はマウントのあいだ1回だけ登録し、アンマウントで解除する", async () => {
  const { update, unmount } = await renderPanel();

  await update({ refreshKey: 1 });
  await update({ open: false });
  await update({ open: true });

  expect(events.onDedupUpdated).toHaveBeenCalledTimes(1);
  expect(unlisten).not.toHaveBeenCalled();

  unmount();

  expect(unlisten).toHaveBeenCalledTimes(1);
});

test("閉じているときは何も描画しないが、取得・購読・件数の通知は行う", async () => {
  const { onCountChange } = await renderPanel({ open: false });

  expect(panel()).toBeNull();
  expect(api.listDuplicateGroups).toHaveBeenCalledTimes(1);
  expect(events.onDedupUpdated).toHaveBeenCalledTimes(1);
  expect(onCountChange).toHaveBeenLastCalledWith(3);
  // 行を描画していないので、サムネイルも取得しない。
  expect(api.getThumbnails).not.toHaveBeenCalled();

  await emit([group([member("p"), member("q")])]);

  expect(onCountChange).toHaveBeenLastCalledWith(2);
  expect(panel()).toBeNull();
});

test("閉じたあとも購読と件数の通知は続き、開き直すと受けた一覧を表示する", async () => {
  const { update, onCountChange } = await renderPanel();

  await update({ open: false });
  await emit([group([member("p"), member("q")])]);

  expect(onCountChange).toHaveBeenLastCalledWith(2);
  expect(unlisten).not.toHaveBeenCalled();

  await update({ open: true });

  expect(paths()).toEqual([pathOf("p"), pathOf("q")]);
});

// ---- 再チェック ----

test("再チェック中は押せず「確認中...」を出し、成功しても dedup:updated が届くまで戻らない", async () => {
  const refresh = deferred<void>();
  vi.mocked(api.refreshDuplicateGroups).mockReturnValue(refresh.promise);
  const { user } = await renderPanel();

  await user.click(refreshButton());

  expect(refreshButton()).toBeDisabled();
  expect(refreshButton()).toHaveTextContent(/^確認中\.\.\.$/);

  await act(async () => {
    refresh.resolve();
  });

  expect(refreshButton()).toBeDisabled();
  expect(refreshButton()).toHaveTextContent("確認中...");

  await emit([group([member("p"), member("q")])]);

  expect(refreshButton()).toBeEnabled();
  expect(refreshButton()).toHaveTextContent(/^重複を再チェック$/);
  expect(paths()).toEqual([pathOf("p"), pathOf("q")]);
  expect(api.refreshDuplicateGroups).toHaveBeenCalledTimes(1);
});

test("再チェックが失敗したら、エラーを出して押せる状態に戻す", async () => {
  vi.mocked(api.refreshDuplicateGroups).mockRejectedValue("dedup failed");
  const { user } = await renderPanel();

  await user.click(refreshButton());

  await waitFor(() => expect(panelError()).toHaveTextContent(/^dedup failed$/));
  expect(refreshButton()).toBeEnabled();
  expect(refreshButton()).toHaveTextContent("重複を再チェック");
});

test("再チェックを始めると、前のエラーを消す", async () => {
  vi.mocked(api.refreshDuplicateGroups)
    .mockRejectedValueOnce("dedup failed")
    .mockReturnValueOnce(never());
  const { user } = await renderPanel();

  await user.click(refreshButton());
  await waitFor(() => expect(panelError()).not.toBeNull());

  await user.click(refreshButton());

  expect(panelError()).toBeNull();
});

// ---- 削除 ----

test("登録解除は確認を出し、確定するまでバックエンドを呼ばない", async () => {
  const { user } = await renderPanel();

  await user.click(unregisterButton("b"));

  const dialog = confirmDialog()!;
  expect(dialog).toBeInTheDocument();
  // ポップオーバーの中ではなく document.body に描画する。
  expect(panel()!.contains(dialog)).toBe(false);
  expect(within(dialog).getByRole("heading")).toHaveTextContent("登録解除");
  expect(dialog.querySelector(".confirm-dialog-path")).toHaveTextContent(pathOf("b"));
  expect(dialog).toHaveTextContent("このカタログ登録を解除します。");
  expect(dialog).toHaveTextContent("元の動画ファイルは削除されません。");
  expect(confirmButton()).toHaveTextContent("登録を解除する");
  expect(confirmButton()).not.toHaveClass("confirm-dialog-confirm--danger");
  expect(api.deleteDuplicateVideo).not.toHaveBeenCalled();
  expect(api.deleteDuplicateVideoAndFile).not.toHaveBeenCalled();
});

test("ファイルを削除は危険な確認を出し、確定するまでバックエンドを呼ばない", async () => {
  const { user } = await renderPanel();

  await user.click(fileDeleteButton("b"));

  const dialog = confirmDialog()!;
  expect(within(dialog).getByRole("heading")).toHaveTextContent("ファイルを削除");
  expect(dialog.querySelector(".confirm-dialog-path")).toHaveTextContent(pathOf("b"));
  expect(dialog).toHaveTextContent("この動画ファイルをゴミ箱に移動し、カタログからも削除します");
  expect(confirmButton()).toHaveTextContent("ゴミ箱に移動して削除");
  expect(confirmButton()).toHaveClass("confirm-dialog-confirm--danger");
  expect(api.deleteDuplicateVideo).not.toHaveBeenCalled();
  expect(api.deleteDuplicateVideoAndFile).not.toHaveBeenCalled();
});

test("確認でキャンセルすると、何も呼ばずに閉じ、行の操作を押せる状態に戻す", async () => {
  const { user } = await renderPanel();

  await user.click(unregisterButton("b"));
  await user.click(cancelButton());

  expect(confirmDialog()).toBeNull();
  expect(api.deleteDuplicateVideo).not.toHaveBeenCalled();
  expect(paths()).toEqual([pathOf("a"), pathOf("b"), pathOf("c")]);
  expect(unregisterButton("a")).toBeEnabled();
});

test("実行中でなければ、背景のクリックと Escape で確認を閉じる", async () => {
  const { user } = await renderPanel();

  await user.click(fileDeleteButton("b"));
  await user.click(confirmDialog()!.parentElement!);
  expect(confirmDialog()).toBeNull();

  await user.click(unregisterButton("b"));
  await user.keyboard("{Escape}");
  expect(confirmDialog()).toBeNull();

  expect(api.deleteDuplicateVideo).not.toHaveBeenCalled();
  expect(api.deleteDuplicateVideoAndFile).not.toHaveBeenCalled();
});

test("登録解除を確定すると、そのメンバーを一覧から外し、件数を通知する", async () => {
  vi.mocked(api.deleteDuplicateVideo).mockResolvedValue(undefined);
  const { user, onCountChange } = await renderPanel();

  await user.click(unregisterButton("b"));
  await user.click(confirmButton());
  await settle();

  expect(api.deleteDuplicateVideo).toHaveBeenCalledWith("b");
  expect(api.deleteDuplicateVideoAndFile).not.toHaveBeenCalled();
  expect(confirmDialog()).toBeNull();
  expect(paths()).toEqual([pathOf("a"), pathOf("c")]);
  expect(onCountChange).toHaveBeenLastCalledWith(2);
  expect(unregisterButton("a")).toBeEnabled();
});

test("ファイルの削除を確定すると、そのメンバーを一覧から外す", async () => {
  vi.mocked(api.deleteDuplicateVideoAndFile).mockResolvedValue(undefined);
  const { user } = await renderPanel();

  await user.click(fileDeleteButton("c"));
  await user.click(confirmButton());
  await settle();

  expect(api.deleteDuplicateVideoAndFile).toHaveBeenCalledWith("c");
  expect(api.deleteDuplicateVideo).not.toHaveBeenCalled();
  expect(confirmDialog()).toBeNull();
  expect(paths()).toEqual([pathOf("a"), pathOf("b")]);
});

test("メンバーが0人になったグループは一覧から外す", async () => {
  vi.mocked(api.deleteDuplicateVideo).mockResolvedValue(undefined);
  const { user } = await renderPanel({
    groups: [group([member("solo")]), group([member("x"), member("y")])],
  });

  await user.click(unregisterButton("solo"));
  await user.click(confirmButton());
  await settle();

  expect(document.querySelectorAll("li.duplicate-group")).toHaveLength(1);
  expect(paths()).toEqual([pathOf("x"), pathOf("y")]);
});

test("最後のグループが無くなったら、空の案内を出す", async () => {
  vi.mocked(api.deleteDuplicateVideo).mockResolvedValue(undefined);
  const { user, onCountChange } = await renderPanel({ groups: [group([member("solo")])] });

  await user.click(unregisterButton("solo"));
  await user.click(confirmButton());
  await settle();

  expect(screen.getByText("重複候補は見つかっていません。")).toBeInTheDocument();
  expect(onCountChange).toHaveBeenLastCalledWith(0);
});

test("実行中は確認のボタンも行の操作も押せず、背景をクリックしても閉じず、二重に実行しない", async () => {
  const removal = deferred<void>();
  vi.mocked(api.deleteDuplicateVideoAndFile).mockReturnValue(removal.promise);
  const { user } = await renderPanel();

  await user.click(fileDeleteButton("b"));
  await user.click(confirmButton());

  expect(confirmButton()).toBeDisabled();
  expect(confirmButton()).toHaveTextContent("処理中...");
  expect(cancelButton()).toBeDisabled();
  for (const button of allRowButtons()) {
    expect(button).toBeDisabled();
  }

  await user.click(confirmButton());
  await user.click(confirmDialog()!.parentElement!);
  await user.keyboard("{Escape}");

  expect(confirmDialog()).toBeInTheDocument();
  expect(api.deleteDuplicateVideoAndFile).toHaveBeenCalledTimes(1);

  await act(async () => {
    removal.resolve();
  });

  expect(confirmDialog()).toBeNull();
  expect(paths()).toEqual([pathOf("a"), pathOf("c")]);
  expect(unregisterButton("a")).toBeEnabled();
  expect(fileDeleteButton("a")).toBeEnabled();
});

test("登録解除が失敗したら、パネルのエラーに出し、一覧は変えずに確認を閉じる", async () => {
  vi.mocked(api.deleteDuplicateVideo).mockRejectedValue("unregister failed");
  const { user } = await renderPanel();

  await user.click(unregisterButton("b"));
  await user.click(confirmButton());
  await settle();

  expect(panelError()).toHaveTextContent(/^unregister failed$/);
  expect(rowErrors("b")).toEqual([]);
  expect(confirmDialog()).toBeNull();
  expect(paths()).toEqual([pathOf("a"), pathOf("b"), pathOf("c")]);
  expect(unregisterButton("b")).toBeEnabled();
});

test("ファイルの削除が失敗したら、その行のエラーに出し、一覧は変えずに確認を閉じる", async () => {
  vi.mocked(api.deleteDuplicateVideoAndFile).mockRejectedValue("trash failed");
  const { user } = await renderPanel();

  await user.click(fileDeleteButton("b"));
  await user.click(confirmButton());
  await settle();

  expect(rowErrors("b")).toEqual(["trash failed"]);
  expect(rowErrors("a")).toEqual([]);
  expect(panelError()).toBeNull();
  expect(confirmDialog()).toBeNull();
  expect(paths()).toEqual([pathOf("a"), pathOf("b"), pathOf("c")]);
  expect(fileDeleteButton("b")).toBeEnabled();
});

test("ファイルを削除を押すと、その行の前回のエラーだけを消してから確認を出す", async () => {
  vi.mocked(api.deleteDuplicateVideoAndFile)
    .mockRejectedValueOnce("fail a")
    .mockRejectedValueOnce("fail b");
  const { user } = await renderPanel();

  await user.click(fileDeleteButton("a"));
  await user.click(confirmButton());
  await settle();
  await user.click(fileDeleteButton("b"));
  await user.click(confirmButton());
  await settle();
  expect(rowErrors("a")).toEqual(["fail a"]);
  expect(rowErrors("b")).toEqual(["fail b"]);

  await user.click(fileDeleteButton("a"));

  expect(confirmDialog()).toBeInTheDocument();
  expect(rowErrors("a")).toEqual([]);
  expect(rowErrors("b")).toEqual(["fail b"]);
});

// 今の動作を固めている: 行のエラーを消すのは「ファイルを削除」を押したときだけ。
test("現状: 登録解除を押しても、その行のファイル削除のエラーは消えない", async () => {
  vi.mocked(api.deleteDuplicateVideoAndFile).mockRejectedValueOnce("fail a");
  const { user } = await renderPanel();

  await user.click(fileDeleteButton("a"));
  await user.click(confirmButton());
  await settle();

  await user.click(unregisterButton("a"));

  expect(confirmDialog()).toBeInTheDocument();
  expect(rowErrors("a")).toEqual(["fail a"]);
});

// ---- 押せない条件 ----

test("確認を出しているあいだは、すべての行の操作と再生を受け付けない", async () => {
  const { user } = await renderPanel();

  await user.click(unregisterButton("a"));

  for (const button of allRowButtons()) {
    expect(button).toBeDisabled();
  }
  await user.dblClick(thumb("b"));
  expect(api.playVideo).not.toHaveBeenCalled();
  // 再チェックは行の操作ではないので止めない。
  expect(refreshButton()).toBeEnabled();
});

test("ファイルを削除できない理由があるときは押せず、その理由を行に出す", async () => {
  await renderPanel({
    groups: [
      group([
        member("online1", { created_at: "2024-01-01 00:00:00" }),
        member("off", { status: "offline", created_at: "2024-01-02 00:00:00" }),
      ]),
      group([member("u1"), member("u2")], "path_collision_unconfirmed"),
    ],
  });

  expect(fileDeleteButton("online1")).toBeDisabled();
  expect(notes("online1")).toEqual(["このグループで到達できる最後の1件のため削除できません"]);
  expect(fileDeleteButton("off")).toBeDisabled();
  expect(notes("off")).toContain("オフラインのため削除できません");
  expect(fileDeleteButton("u1")).toBeDisabled();
  expect(notes("u1")).toEqual(["内容の一致が確認できていないため削除できません"]);
  // 登録解除は理由に関わらず押せる。
  expect(unregisterButton("online1")).toBeEnabled();
  expect(unregisterButton("off")).toBeEnabled();
  expect(unregisterButton("u1")).toBeEnabled();
  expect(fileDeleteButton("u1")).toHaveAttribute(
    "title",
    "ファイルをゴミ箱に移動し、カタログからも削除します",
  );
});

// ---- 件数の通知 ----

test("件数は、マウント時・取得・イベント・削除でメンバーの合計が変わるたびに通知し、変わらなければ通知しない", async () => {
  vi.mocked(api.deleteDuplicateVideo).mockResolvedValue(undefined);
  const { user, onCountChange, update } = await renderPanel();

  expect(onCountChange.mock.calls).toEqual([[0], [3]]);

  // 合計が同じなら呼ばない。
  await emit([group([member("p"), member("q"), member("r")])]);
  expect(onCountChange.mock.calls).toEqual([[0], [3]]);

  await emit([group([member("p"), member("q"), member("r")]), group([member("s"), member("t")])]);
  expect(onCountChange).toHaveBeenLastCalledWith(5);

  await user.click(unregisterButton("s"));
  await user.click(confirmButton());
  await settle();
  expect(onCountChange).toHaveBeenLastCalledWith(4);

  vi.mocked(api.listDuplicateGroups).mockResolvedValue([group([member("x"), member("y")])]);
  await update({ refreshKey: 1 });
  expect(onCountChange).toHaveBeenLastCalledWith(2);
  expect(onCountChange.mock.calls).toEqual([[0], [3], [5], [4], [2]]);
});

// ---- 閉じたとき ----

test("閉じると保留中の確認を捨て、開き直しても確認は出ず、行の操作を押せる", async () => {
  const { user, update } = await renderPanel();

  await user.click(fileDeleteButton("b"));
  expect(confirmDialog()).toBeInTheDocument();

  await update({ open: false });
  expect(confirmDialog()).toBeNull();
  await update({ open: true });

  expect(confirmDialog()).toBeNull();
  expect(fileDeleteButton("b")).toBeEnabled();
  expect(api.deleteDuplicateVideoAndFile).not.toHaveBeenCalled();
});

test("実行中に閉じても保留は捨てず、開き直すと実行中の確認が残っていて、終われば閉じる", async () => {
  const removal = deferred<void>();
  vi.mocked(api.deleteDuplicateVideo).mockReturnValue(removal.promise);
  const { user, update, onCountChange } = await renderPanel();

  await user.click(unregisterButton("b"));
  await user.click(confirmButton());
  await update({ open: false });
  await update({ open: true });

  expect(confirmDialog()).toBeInTheDocument();
  expect(confirmButton()).toHaveTextContent("処理中...");

  await act(async () => {
    removal.resolve();
  });

  expect(confirmDialog()).toBeNull();
  expect(paths()).toEqual([pathOf("a"), pathOf("c")]);
  expect(onCountChange).toHaveBeenLastCalledWith(2);
});

test("閉じているあいだに実行が終わっても、結果は一覧と件数に反映する", async () => {
  const removal = deferred<void>();
  vi.mocked(api.deleteDuplicateVideo).mockReturnValue(removal.promise);
  const { user, update, onCountChange } = await renderPanel();

  await user.click(unregisterButton("b"));
  await user.click(confirmButton());
  await update({ open: false });
  await act(async () => {
    removal.resolve();
  });
  expect(onCountChange).toHaveBeenLastCalledWith(2);

  await update({ open: true });

  expect(confirmDialog()).toBeNull();
  expect(paths()).toEqual([pathOf("a"), pathOf("c")]);
});

// ---- メンバー1行: サムネイル ----

test("サムネイルは行ごとに video_id で取得し、先頭の1枚だけを出す", async () => {
  vi.mocked(api.getThumbnails).mockImplementation(async (id) => [
    `data:${id}-0`,
    `data:${id}-1`,
  ]);
  await renderPanel();

  expect(vi.mocked(api.getThumbnails).mock.calls.map((c) => c[0]).sort()).toEqual(["a", "b", "c"]);
  const images = row("a").querySelectorAll("img");
  expect(images).toHaveLength(1);
  expect(images[0]).toHaveAttribute("src", "data:a-0");
  expect(images[0]).toHaveAttribute("alt", "a.mp4");
  expect(images[0]).toHaveAttribute("loading", "lazy");
  expect(row("a").querySelector(".thumbnail-placeholder")).toBeNull();
});

test("サムネイルが無い・空・取得失敗のときは「サムネイルなし」を出す", async () => {
  vi.mocked(api.getThumbnails).mockImplementation(async (id) => {
    if (id === "a") return null;
    if (id === "b") return [];
    throw new Error("thumb failed");
  });
  await renderPanel();

  for (const id of ["a", "b", "c"]) {
    expect(row(id).querySelector("img")).toBeNull();
    expect(row(id).querySelector(".thumbnail-placeholder")).toHaveTextContent(/^サムネイルなし$/);
  }
});

test("一覧が変わって video_id が変わったら取得し直し、古い取得の結果は表示しない", async () => {
  const stale = deferred<string[] | null>();
  vi.mocked(api.getThumbnails).mockImplementation((id) =>
    id === "old" ? stale.promise : Promise.resolve([`data:${id}`]),
  );
  const { update } = await renderPanel({ groups: [group([member("old")])] });

  vi.mocked(api.listDuplicateGroups).mockResolvedValue([group([member("new")])]);
  await update({ refreshKey: 1 });
  expect(row("new").querySelector("img")).toHaveAttribute("src", "data:new");

  await act(async () => {
    stale.resolve(["data:old"]);
  });

  expect(rows()).toHaveLength(1);
  expect(row("new").querySelector("img")).toHaveAttribute("src", "data:new");
  expect(document.querySelector('img[src="data:old"]')).toBeNull();
  expect(vi.mocked(api.getThumbnails).mock.calls.map((c) => c[0])).toEqual(["old", "new"]);
});

test("同じ video_id のまま一覧が置き換わっても、サムネイルは取得し直さない", async () => {
  vi.mocked(api.getThumbnails).mockImplementation(async (id) => [`data:${id}`]);
  await renderPanel();

  await emit(THREE());

  expect(api.getThumbnails).toHaveBeenCalledTimes(3);
  expect(row("a").querySelector("img")).toHaveAttribute("src", "data:a");
});

// ---- メンバー1行: オフラインと再生 ----

test("オンラインの行は、ダブルクリックでそのパスを再生する", async () => {
  const { user } = await renderPanel();

  expect(row("a")).toHaveAttribute("class", "duplicate-group-member");
  await user.dblClick(thumb("a"));

  expect(api.playVideo).toHaveBeenCalledWith(pathOf("a"));
  expect(api.playVideo).toHaveBeenCalledTimes(1);
});

test("オフラインの行は表示を変えて案内を出し、ダブルクリックしても再生しない", async () => {
  const { user } = await renderPanel({
    groups: [group([member("a"), member("off", { status: "offline" })])],
  });

  expect(row("off")).toHaveAttribute(
    "class",
    "duplicate-group-member duplicate-group-member--offline",
  );
  expect(row("off").querySelector(".thumbnail-placeholder")).toHaveTextContent(/^オフライン$/);
  expect(row("off").querySelector(".thumbnail-offline-badge")).toHaveTextContent("オフライン");
  expect(notes("off")).toContain("オフラインのため再生できません");
  expect(row("a").querySelector(".thumbnail-offline-badge")).toBeNull();

  await user.dblClick(thumb("off"));

  expect(api.playVideo).not.toHaveBeenCalled();
});

test("保存先のフォルダに届かない行は、status が online でもオフラインとして扱う", async () => {
  const { user } = await renderPanel({
    groups: [group([member("a"), member("far", { file_path: "N:\\nas\\far.mp4" })])],
    unreachableFolders: ["n:/NAS"],
  });

  const far = rows().find(
    (r) => r.querySelector(".duplicate-group-member-path")?.textContent === "N:\\nas\\far.mp4",
  )!;
  expect(far).toHaveClass("duplicate-group-member--offline");
  expect(far.querySelector(".thumbnail-placeholder")).toHaveTextContent(/^オフライン$/);
  expect(far.querySelector(".thumbnail-offline-badge")).not.toBeNull();
  expect(far).toHaveTextContent("オフラインのため再生できません");
  expect(row("a")).not.toHaveClass("duplicate-group-member--offline");

  await user.dblClick(far.querySelector<HTMLElement>(".duplicate-group-member-thumb")!);

  expect(api.playVideo).not.toHaveBeenCalled();
});

test("再生の開始中は案内を出し、もう一度ダブルクリックしても二重に起動しない", async () => {
  const play = deferred<void>();
  vi.mocked(api.playVideo).mockReturnValue(play.promise);
  const { user } = await renderPanel();

  await user.dblClick(thumb("a"));

  expect(notes("a")).toEqual(["再生を開始しています…"]);
  expect(notes("b")).toEqual([]);

  await user.dblClick(thumb("a"));
  expect(api.playVideo).toHaveBeenCalledTimes(1);

  await act(async () => {
    play.resolve();
  });

  expect(notes("a")).toEqual([]);
  expect(rowErrors("a")).toEqual([]);

  await user.dblClick(thumb("a"));
  expect(api.playVideo).toHaveBeenCalledTimes(2);
});

test("再生の状態は行の位置ではなくメンバーに付き、前の行が消えても同じメンバーの行に残る", async () => {
  const play = deferred<void>();
  vi.mocked(api.playVideo).mockReturnValue(play.promise);
  vi.mocked(api.deleteDuplicateVideo).mockResolvedValue(undefined);
  const { user } = await renderPanel();

  await user.dblClick(thumb("b"));
  expect(notes("b")).toEqual(["再生を開始しています…"]);

  await user.click(unregisterButton("a"));
  await user.click(confirmButton());
  await settle();

  expect(paths()).toEqual([pathOf("b"), pathOf("c")]);
  expect(notes("b")).toEqual(["再生を開始しています…"]);
  expect(notes("c")).toEqual([]);
});

test("再生に失敗したら理由を行に出し、次に再生を始めるときに消す", async () => {
  vi.mocked(api.playVideo).mockRejectedValueOnce("no player").mockReturnValueOnce(never());
  const { user } = await renderPanel();

  await user.dblClick(thumb("a"));
  await settle();

  expect(rowErrors("a")).toEqual(["再生に失敗しました: no player"]);
  expect(notes("a")).toEqual([]);

  await user.dblClick(thumb("a"));

  expect(rowErrors("a")).toEqual([]);
  expect(notes("a")).toEqual(["再生を開始しています…"]);
});

test("削除の実行中は、ダブルクリックしても再生しない", async () => {
  vi.mocked(api.deleteDuplicateVideo).mockReturnValue(never());
  const { user } = await renderPanel();

  await user.click(unregisterButton("c"));
  await user.click(confirmButton());
  await user.dblClick(thumb("a"));

  expect(api.playVideo).not.toHaveBeenCalled();
});

// ---- グループ ----

test("メンバーは登録日時の古い順に並べる", async () => {
  await renderPanel({
    groups: [
      group([
        member("c", { created_at: "2024-03-01 00:00:00" }),
        member("a", { created_at: "2024-01-01 00:00:00" }),
        member("b", { created_at: "2024-02-01 00:00:00" }),
      ]),
    ],
  });

  expect(paths()).toEqual([pathOf("a"), pathOf("b"), pathOf("c")]);
  expect(Array.from(row("b").querySelectorAll("dd")).map((dd) => dd.textContent)).toEqual([
    "100.0 MB",
    "1:00",
    "1920 × 1080",
    "2024-02-01 00:00:00",
  ]);
});

test("グループ内で値が違う項目にだけ差分の class を付ける", async () => {
  await renderPanel({
    groups: [
      group([
        member("a", { file_size: 1024 * 1024 * 1024 * 2 }),
        member("b", { width: null, height: null }),
      ]),
      group([member("x"), member("y")]),
    ],
  });

  const ddClasses = (id: string) =>
    Array.from(row(id).querySelectorAll("dd")).map((dd) => dd.getAttribute("class"));
  expect(ddClasses("a")).toEqual([
    "duplicate-group-member-diff",
    null,
    "duplicate-group-member-diff",
    null,
  ]);
  expect(row("a").querySelector("dd")).toHaveTextContent("2.00 GB");
  expect(row("b").querySelectorAll("dd")[2]).toHaveTextContent("不明");
  expect(ddClasses("x")).toEqual([null, null, null, null]);
});

test("メンバーが1件のグループは、差分を付けない", async () => {
  await renderPanel({ groups: [group([member("solo", { duration: null })])] });

  expect(
    Array.from(row("solo").querySelectorAll("dd")).map((dd) => dd.getAttribute("class")),
  ).toEqual([null, null, null, null]);
});

test("種別のラベルを出し、未確認の候補にだけ注記を出す", async () => {
  await renderPanel({
    groups: [
      group([member("k1"), member("k2")], "quick_hash_confirmed"),
      group([member("k3"), member("k4")], "path_collision_confirmed"),
      group([member("k5"), member("k6")], "path_collision_unconfirmed"),
      group([member("k7"), member("k8")], "quick_hash_candidate_unverified"),
    ],
  });

  const headers = Array.from(document.querySelectorAll(".duplicate-group-header"));
  expect(headers.map((h) => h.querySelector(".duplicate-group-kind")?.textContent)).toEqual([
    "内容一致（確定）",
    "パス衝突・内容確認済み",
    "パス衝突・内容未確認",
    "候補・内容未確認",
  ]);
  expect(
    headers.map((h) => h.querySelector(".duplicate-group-kind-note")?.textContent ?? null),
  ).toEqual([null, null, null, "NAS に接続できないため、内容が同じか確認できません"]);
});
