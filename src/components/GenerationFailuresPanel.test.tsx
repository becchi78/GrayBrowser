// GenerationFailuresPanel の現状の動作を固定する特性テスト。
import { beforeEach, expect, test, vi } from "vitest";
import { act, screen, waitFor, within } from "@testing-library/react";
import * as api from "../api";
import * as events from "../events";
import { UnreachableFoldersContext } from "../reachabilityContext";
import { renderWithProviders } from "../testing/render";
import type { ExhaustedMetadataDto, ExhaustedThumbnailDto, GenerationFailuresDto } from "../types";
import { GenerationFailuresPanel } from "./GenerationFailuresPanel";

vi.mock("../api", () => ({
  countMissingMetadata: vi.fn(),
  countMissingThumbnails: vi.fn(),
  listGenerationFailures: vi.fn(),
  regenerateMissingThumbnails: vi.fn(),
  reprobeMissingMetadata: vi.fn(),
  retryMetadataProbe: vi.fn(),
  retryThumbnailGeneration: vi.fn(),
}));

vi.mock("../events", () => ({
  onCatalogChanged: vi.fn(),
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

const pathOf = (name: string) => `C:\\videos\\sub\\${name}.mp4`;

function thumbFail(
  id: string,
  overrides: Partial<ExhaustedThumbnailDto> = {},
): ExhaustedThumbnailDto {
  return {
    id,
    file_path: pathOf(id),
    file_name: `${id}.mp4`,
    thumbnail_attempts: 3,
    ...overrides,
  };
}

function metaFail(id: string, overrides: Partial<ExhaustedMetadataDto> = {}): ExhaustedMetadataDto {
  return {
    id,
    file_path: pathOf(id),
    file_name: `${id}.mp4`,
    metadata_attempts: 3,
    ...overrides,
  };
}

function failuresOf(
  thumbnail_failures: ExhaustedThumbnailDto[] = [],
  metadata_failures: ExhaustedMetadataDto[] = [],
): GenerationFailuresDto {
  return { thumbnail_failures, metadata_failures };
}

/** サムネイル失敗2件・メタデータ失敗1件。 */
const DEFAULT = () => failuresOf([thumbFail("t1"), thumbFail("t2")], [metaFail("m1")]);

const THUMB_BADGE = "サムネイル生成失敗";
const META_BADGE = "メタデータ取得失敗";

// catalog:changed の購読に渡されたコールバックと、その解除関数。
let emitCatalog: (() => void) | null = null;
let unlisten = vi.fn();

beforeEach(() => {
  // 失敗の経路では console.error に出すだけなので、テストの出力を汚さないよう黙らせる。
  vi.spyOn(console, "error").mockImplementation(() => {});
  emitCatalog = null;
  unlisten = vi.fn();
  vi.mocked(events.onCatalogChanged).mockImplementation((cb) => {
    emitCatalog = cb;
    return unlisten;
  });
  vi.mocked(api.retryThumbnailGeneration).mockResolvedValue(undefined);
  vi.mocked(api.retryMetadataProbe).mockResolvedValue(undefined);
});

/** 連なった Promise の解決と、それに続く再描画を落ち着かせる。 */
async function settle() {
  for (let i = 0; i < 3; i++) {
    await act(async () => {});
  }
}

/** catalog:changed を届け、取り直しの結果が描画されるまで待つ。 */
async function emitCatalogChanged() {
  await act(async () => {
    emitCatalog!();
  });
  await settle();
}

interface PanelProps {
  refreshKey: number;
  open: boolean;
}

async function renderPanel({
  failures = DEFAULT(),
  open = true,
}: {
  failures?: GenerationFailuresDto;
  open?: boolean;
} = {}) {
  vi.mocked(api.listGenerationFailures).mockResolvedValue(failures);
  const onCountChange = vi.fn();
  let current: PanelProps = { refreshKey: 0, open };
  const ui = (p: PanelProps) => (
    <GenerationFailuresPanel refreshKey={p.refreshKey} open={p.open} onCountChange={onCountChange} />
  );
  const view = renderWithProviders(ui(current));
  await waitFor(() => expect(api.listGenerationFailures).toHaveBeenCalled());
  await settle();
  // rerender は根の要素を差し替えるので、初回描画と同じ Provider で包まないと
  // 作り直されて状態が消え、閉じても状態が残ることを前提にした確認ができない。
  const update = async (next: Partial<PanelProps>) => {
    current = { ...current, ...next };
    view.rerender(
      <UnreachableFoldersContext.Provider value={[]}>{ui(current)}</UnreachableFoldersContext.Provider>,
    );
    await settle();
  };
  return { ...view, onCountChange, update };
}

const panel = () => screen.queryByTestId("status-panel");
const items = () => Array.from(document.querySelectorAll<HTMLElement>("li.generation-failure-item"));
const badgeOf = (li: HTMLElement) => li.querySelector(".generation-failure-badge")?.textContent;
const nameOf = (li: HTMLElement) => li.querySelector(".generation-failure-name")?.textContent;
/** 並び順どおりの「バッジ/ファイル名」。 */
const listing = () => items().map((li) => `${badgeOf(li)}/${nameOf(li)}`);
const row = (badge: string, fileName: string) => {
  const found = items().find((li) => badgeOf(li) === badge && nameOf(li) === fileName);
  if (!found) {
    throw new Error(`行が見つからない: ${badge} ${fileName}`);
  }
  return found;
};
const thumbRow = (id: string) => row(THUMB_BADGE, `${id}.mp4`);
const metaRow = (id: string) => row(META_BADGE, `${id}.mp4`);
const retryButton = (li: HTMLElement) => within(li).getByRole("button", { name: "再試行" });
const panelError = () => document.querySelector(".generation-failures-error");
const notice = () => screen.queryByTestId("regenerate-missing-notice");
const thumbBulkButton = () => screen.getByTestId("regenerate-missing-thumbnails-btn");
const metaBulkButton = () => screen.getByTestId("reprobe-missing-metadata-btn");
const confirmDialog = () => screen.queryByTestId("confirm-dialog");
const confirmButton = () => screen.getByTestId("confirm-dialog-confirm-btn");
const cancelButton = () => screen.getByTestId("confirm-dialog-cancel-btn");

// ---- 一覧の取得と購読 ----

test("開くと一覧を取得し、サムネイル失敗・メタデータ失敗の行を表示する", async () => {
  await renderPanel();

  expect(panel()).toHaveClass("generation-failures-panel", "status-panel");
  expect(panel()!.tagName).toBe("SECTION");
  expect(document.querySelector("ul.generation-failures-list")).not.toBeNull();
  expect(listing()).toEqual([
    `${THUMB_BADGE}/t1.mp4`,
    `${THUMB_BADGE}/t2.mp4`,
    `${META_BADGE}/m1.mp4`,
  ]);
  expect(api.listGenerationFailures).toHaveBeenCalledTimes(1);
  expect(panelError()).toBeNull();
});

test("refreshKey が変わるたびに取得し直し、変わらなければ取得しない", async () => {
  const { update } = await renderPanel();

  await update({ open: true });
  expect(api.listGenerationFailures).toHaveBeenCalledTimes(1);

  vi.mocked(api.listGenerationFailures).mockResolvedValue(failuresOf([thumbFail("x")]));
  await update({ refreshKey: 1 });

  expect(api.listGenerationFailures).toHaveBeenCalledTimes(2);
  expect(listing()).toEqual([`${THUMB_BADGE}/x.mp4`]);

  vi.mocked(api.listGenerationFailures).mockResolvedValue(failuresOf([], [metaFail("y")]));
  await update({ refreshKey: 2 });

  expect(api.listGenerationFailures).toHaveBeenCalledTimes(3);
  expect(listing()).toEqual([`${META_BADGE}/y.mp4`]);
});

test("catalog:changed を受けるたびに取得し直して一覧を置き換える", async () => {
  await renderPanel();

  vi.mocked(api.listGenerationFailures).mockResolvedValue(failuresOf([thumbFail("p")]));
  await emitCatalogChanged();

  expect(api.listGenerationFailures).toHaveBeenCalledTimes(2);
  expect(listing()).toEqual([`${THUMB_BADGE}/p.mp4`]);

  vi.mocked(api.listGenerationFailures).mockResolvedValue(failuresOf());
  await emitCatalogChanged();

  expect(api.listGenerationFailures).toHaveBeenCalledTimes(3);
  expect(items()).toEqual([]);
  expect(screen.getByText("生成に失敗したファイルはありません。")).toBeInTheDocument();
});

test("購読はマウントのあいだ1回だけ登録し、アンマウントで解除する", async () => {
  const { update, unmount } = await renderPanel();

  await update({ refreshKey: 1 });
  await update({ open: false });
  await update({ open: true });

  expect(events.onCatalogChanged).toHaveBeenCalledTimes(1);
  expect(unlisten).not.toHaveBeenCalled();

  unmount();

  expect(unlisten).toHaveBeenCalledTimes(1);
});

test("閉じているときは何も描画しないが、取得・購読・件数の通知は行う", async () => {
  const { onCountChange, update } = await renderPanel({ open: false });

  expect(panel()).toBeNull();
  expect(api.listGenerationFailures).toHaveBeenCalledTimes(1);
  expect(events.onCatalogChanged).toHaveBeenCalledTimes(1);
  expect(onCountChange).toHaveBeenLastCalledWith(3);

  vi.mocked(api.listGenerationFailures).mockResolvedValue(failuresOf([thumbFail("p")]));
  await emitCatalogChanged();

  expect(api.listGenerationFailures).toHaveBeenCalledTimes(2);
  expect(onCountChange).toHaveBeenLastCalledWith(1);
  expect(panel()).toBeNull();

  vi.mocked(api.listGenerationFailures).mockResolvedValue(
    failuresOf([thumbFail("p")], [metaFail("q")]),
  );
  await update({ refreshKey: 1 });

  expect(api.listGenerationFailures).toHaveBeenCalledTimes(3);
  expect(onCountChange).toHaveBeenLastCalledWith(2);
  expect(panel()).toBeNull();

  await update({ open: true });

  expect(listing()).toEqual([`${THUMB_BADGE}/p.mp4`, `${META_BADGE}/q.mp4`]);
});

test("一覧の取得が失敗したら、エラーを出し、空の案内を出す", async () => {
  vi.mocked(api.listGenerationFailures).mockRejectedValueOnce("list failed");
  await renderPanel();

  expect(panelError()).toHaveTextContent(/^list failed$/);
  expect(screen.getByText("生成に失敗したファイルはありません。")).toBeInTheDocument();
});

test("catalog:changed での取り直しが失敗したら、エラーを出し、前の一覧は残す", async () => {
  await renderPanel();

  vi.mocked(api.listGenerationFailures).mockRejectedValueOnce("refresh failed");
  await emitCatalogChanged();

  expect(panelError()).toHaveTextContent(/^refresh failed$/);
  expect(items()).toHaveLength(3);
});

// ---- 再試行 ----

test("サムネイルの再試行は retryThumbnailGeneration を id で呼ぶ", async () => {
  const { user } = await renderPanel();

  await user.click(retryButton(thumbRow("t2")));
  await settle();

  expect(api.retryThumbnailGeneration).toHaveBeenCalledTimes(1);
  expect(api.retryThumbnailGeneration).toHaveBeenCalledWith("t2");
  expect(api.retryMetadataProbe).not.toHaveBeenCalled();
});

test("メタデータの再試行は retryMetadataProbe を id で呼ぶ", async () => {
  const { user } = await renderPanel();

  await user.click(retryButton(metaRow("m1")));
  await settle();

  expect(api.retryMetadataProbe).toHaveBeenCalledTimes(1);
  expect(api.retryMetadataProbe).toHaveBeenCalledWith("m1");
  expect(api.retryThumbnailGeneration).not.toHaveBeenCalled();
});

test("再試行中はその行の再試行だけ押せず、終わったら押せる状態に戻る", async () => {
  const retry = deferred<void>();
  vi.mocked(api.retryThumbnailGeneration).mockReturnValue(retry.promise);
  const { user } = await renderPanel();

  await user.click(retryButton(thumbRow("t1")));

  expect(retryButton(thumbRow("t1"))).toBeDisabled();
  expect(retryButton(thumbRow("t2"))).toBeEnabled();
  expect(retryButton(metaRow("m1"))).toBeEnabled();
  // 一括の操作は再試行と無関係。
  expect(thumbBulkButton()).toBeEnabled();
  expect(metaBulkButton()).toBeEnabled();

  await act(async () => {
    retry.resolve();
  });

  expect(retryButton(thumbRow("t1"))).toBeEnabled();
});

test("同じ id のサムネイル失敗とメタデータ失敗は別の行として、片方の再試行中も他方を押せる", async () => {
  const thumbRetry = deferred<void>();
  const metaRetry = deferred<void>();
  vi.mocked(api.retryThumbnailGeneration).mockReturnValue(thumbRetry.promise);
  vi.mocked(api.retryMetadataProbe).mockReturnValue(metaRetry.promise);
  const { user } = await renderPanel({
    failures: failuresOf([thumbFail("same")], [metaFail("same")]),
  });

  expect(listing()).toEqual([`${THUMB_BADGE}/same.mp4`, `${META_BADGE}/same.mp4`]);
  // いまは種類ごとに別の配列で描画しているので、key の重なりは起こらない。両方の種類を
  // 1つの配列にまとめたうえで key を id だけにすると、React が重複 key の警告を
  // console.error に出す(表示は崩れない)。この確認が捕まえるのは、その組み合わせだけ。
  expect(console.error).not.toHaveBeenCalled();

  await user.click(retryButton(thumbRow("same")));

  expect(retryButton(thumbRow("same"))).toBeDisabled();
  expect(retryButton(metaRow("same"))).toBeEnabled();

  await user.click(retryButton(metaRow("same")));

  expect(retryButton(metaRow("same"))).toBeDisabled();
  expect(api.retryThumbnailGeneration).toHaveBeenCalledWith("same");
  expect(api.retryMetadataProbe).toHaveBeenCalledWith("same");

  await act(async () => {
    thumbRetry.resolve();
  });

  // 終わった方だけが戻り、もう片方は実行中のまま。
  expect(retryButton(thumbRow("same"))).toBeEnabled();
  expect(retryButton(metaRow("same"))).toBeDisabled();

  await act(async () => {
    metaRetry.resolve();
  });

  expect(retryButton(metaRow("same"))).toBeEnabled();
});

test("再試行が成功しても一覧はその場では変わらず、catalog:changed で変わる", async () => {
  const { user } = await renderPanel();

  vi.mocked(api.listGenerationFailures).mockResolvedValue(
    failuresOf([thumbFail("t2")], [metaFail("m1")]),
  );
  await user.click(retryButton(thumbRow("t1")));
  await settle();

  expect(api.listGenerationFailures).toHaveBeenCalledTimes(1);
  expect(listing()).toEqual([
    `${THUMB_BADGE}/t1.mp4`,
    `${THUMB_BADGE}/t2.mp4`,
    `${META_BADGE}/m1.mp4`,
  ]);
  expect(retryButton(thumbRow("t1"))).toBeEnabled();
  expect(panelError()).toBeNull();

  await emitCatalogChanged();

  expect(listing()).toEqual([`${THUMB_BADGE}/t2.mp4`, `${META_BADGE}/m1.mp4`]);
});

test("再試行が失敗したら、エラーを出し、押せる状態に戻る", async () => {
  vi.mocked(api.retryMetadataProbe).mockRejectedValue("probe failed");
  const { user } = await renderPanel();

  await user.click(retryButton(metaRow("m1")));
  await settle();

  expect(panelError()).toHaveTextContent(/^probe failed$/);
  expect(retryButton(metaRow("m1"))).toBeEnabled();
  expect(items()).toHaveLength(3);
});

// ---- 一括の再作成・再取得 ----

test("数えているあいだは、一括の2つのボタンとも押せない", async () => {
  const count = deferred<number>();
  vi.mocked(api.countMissingThumbnails).mockReturnValue(count.promise);
  const { user } = await renderPanel();

  await user.click(thumbBulkButton());

  expect(thumbBulkButton()).toBeDisabled();
  expect(metaBulkButton()).toBeDisabled();
  expect(confirmDialog()).toBeNull();

  await act(async () => {
    count.resolve(0);
  });

  expect(thumbBulkButton()).toBeEnabled();
  expect(metaBulkButton()).toBeEnabled();
});

test("サムネイルの件数が0なら確認を出さず、その案内を出して実行しない", async () => {
  vi.mocked(api.countMissingThumbnails).mockResolvedValue(0);
  const { user } = await renderPanel();

  await user.click(thumbBulkButton());
  await settle();

  expect(api.countMissingThumbnails).toHaveBeenCalledTimes(1);
  expect(api.countMissingMetadata).not.toHaveBeenCalled();
  expect(confirmDialog()).toBeNull();
  expect(notice()).toHaveTextContent(/^再作成が必要なサムネイルはありません。$/);
  expect(notice()).toHaveClass("generation-failures-notice");
  expect(api.regenerateMissingThumbnails).not.toHaveBeenCalled();
});

test("メタデータの件数が0なら確認を出さず、その案内を出して実行しない", async () => {
  vi.mocked(api.countMissingMetadata).mockResolvedValue(0);
  const { user } = await renderPanel();

  await user.click(metaBulkButton());
  await settle();

  expect(api.countMissingMetadata).toHaveBeenCalledTimes(1);
  expect(api.countMissingThumbnails).not.toHaveBeenCalled();
  expect(confirmDialog()).toBeNull();
  expect(notice()).toHaveTextContent(/^再取得が必要なメタデータはありません。$/);
  expect(api.reprobeMissingMetadata).not.toHaveBeenCalled();
});

test("一括のボタンを押すと、まず前回の案内を消す", async () => {
  vi.mocked(api.countMissingMetadata).mockResolvedValueOnce(0).mockReturnValueOnce(never());
  vi.mocked(api.countMissingThumbnails).mockReturnValue(never());
  const { user } = await renderPanel();

  await user.click(metaBulkButton());
  await settle();
  expect(notice()).not.toBeNull();

  // 数える処理が終わらなくても、押した時点で消える。
  await user.click(metaBulkButton());
  expect(notice()).toBeNull();
});

test("別の一括のボタンを押しても、前回の案内を消す", async () => {
  vi.mocked(api.countMissingThumbnails).mockResolvedValueOnce(0);
  vi.mocked(api.countMissingMetadata).mockResolvedValueOnce(5);
  const { user } = await renderPanel();

  await user.click(thumbBulkButton());
  await settle();
  expect(notice()).toHaveTextContent("再作成が必要なサムネイルはありません。");

  await user.click(metaBulkButton());
  await settle();

  expect(notice()).toBeNull();
  expect(confirmDialog()).toBeInTheDocument();
});

test("サムネイルの件数が1件以上なら、件数入りの確認を出し、確定するまで実行しない", async () => {
  vi.mocked(api.countMissingThumbnails).mockResolvedValue(7);
  const { user } = await renderPanel();

  await user.click(thumbBulkButton());
  await settle();

  const dialog = confirmDialog()!;
  expect(dialog).toBeInTheDocument();
  expect(panel()!.contains(dialog)).toBe(false);
  expect(within(dialog).getByRole("heading")).toHaveTextContent(/^サムネイルの再作成$/);
  expect(dialog).toHaveAttribute("aria-label", "サムネイルの再作成");
  expect(dialog.querySelector(".confirm-dialog-body")).toHaveTextContent(
    /^7 件の動画のサムネイルを再作成します。時間がかかることがあります。$/,
  );
  expect(confirmButton()).toHaveTextContent(/^再作成する$/);
  expect(notice()).toBeNull();
  expect(api.regenerateMissingThumbnails).not.toHaveBeenCalled();
  expect(api.reprobeMissingMetadata).not.toHaveBeenCalled();
});

test("メタデータの件数が1件以上なら、件数入りの確認を出し、確定するまで実行しない", async () => {
  vi.mocked(api.countMissingMetadata).mockResolvedValue(12);
  const { user } = await renderPanel();

  await user.click(metaBulkButton());
  await settle();

  const dialog = confirmDialog()!;
  expect(within(dialog).getByRole("heading")).toHaveTextContent(/^メタデータの再取得$/);
  expect(dialog.querySelector(".confirm-dialog-body")).toHaveTextContent(
    /^12 件の動画のメタデータ\(解像度など\)を再取得します。$/,
  );
  expect(confirmButton()).toHaveTextContent(/^再取得する$/);
  expect(api.reprobeMissingMetadata).not.toHaveBeenCalled();
  expect(api.regenerateMissingThumbnails).not.toHaveBeenCalled();
});

test("サムネイルの確認を確定すると regenerateMissingThumbnails を呼び、実行中は押せず、終わったら確認を閉じる", async () => {
  const run = deferred<number>();
  vi.mocked(api.countMissingThumbnails).mockResolvedValue(2);
  vi.mocked(api.regenerateMissingThumbnails).mockReturnValue(run.promise);
  const { user } = await renderPanel();

  await user.click(thumbBulkButton());
  await settle();
  await user.click(confirmButton());

  expect(api.regenerateMissingThumbnails).toHaveBeenCalledTimes(1);
  expect(api.reprobeMissingMetadata).not.toHaveBeenCalled();
  expect(thumbBulkButton()).toBeDisabled();
  expect(metaBulkButton()).toBeDisabled();
  expect(confirmDialog()).toBeInTheDocument();
  expect(confirmButton()).toBeDisabled();
  expect(confirmButton()).toHaveTextContent("処理中...");
  expect(cancelButton()).toBeDisabled();

  await act(async () => {
    run.resolve(2);
  });

  expect(confirmDialog()).toBeNull();
  expect(thumbBulkButton()).toBeEnabled();
  expect(metaBulkButton()).toBeEnabled();
  expect(panelError()).toBeNull();
  // 一覧は catalog:changed で更新されるので、ここでは取り直さない。
  expect(api.listGenerationFailures).toHaveBeenCalledTimes(1);
});

test("メタデータの確認を確定すると reprobeMissingMetadata を呼び、終わったら確認を閉じる", async () => {
  vi.mocked(api.countMissingMetadata).mockResolvedValue(4);
  vi.mocked(api.reprobeMissingMetadata).mockResolvedValue(4);
  const { user } = await renderPanel();

  await user.click(metaBulkButton());
  await settle();
  await user.click(confirmButton());
  await settle();

  expect(api.reprobeMissingMetadata).toHaveBeenCalledTimes(1);
  expect(api.regenerateMissingThumbnails).not.toHaveBeenCalled();
  expect(confirmDialog()).toBeNull();
  expect(metaBulkButton()).toBeEnabled();
});

test("実行が失敗したら、エラーを出し、確認も閉じる", async () => {
  vi.mocked(api.countMissingThumbnails).mockResolvedValue(2);
  vi.mocked(api.regenerateMissingThumbnails).mockRejectedValue("regen failed");
  const { user } = await renderPanel();

  await user.click(thumbBulkButton());
  await settle();
  await user.click(confirmButton());
  await settle();

  expect(panelError()).toHaveTextContent(/^regen failed$/);
  expect(confirmDialog()).toBeNull();
  expect(thumbBulkButton()).toBeEnabled();
  expect(metaBulkButton()).toBeEnabled();
});

test("数える処理が失敗したら、エラーを出し、確認も案内も出さず、押せる状態に戻る", async () => {
  vi.mocked(api.countMissingMetadata).mockRejectedValue("count failed");
  const { user } = await renderPanel();

  await user.click(metaBulkButton());
  await settle();

  expect(panelError()).toHaveTextContent(/^count failed$/);
  expect(confirmDialog()).toBeNull();
  expect(notice()).toBeNull();
  expect(thumbBulkButton()).toBeEnabled();
  expect(metaBulkButton()).toBeEnabled();
  expect(api.reprobeMissingMetadata).not.toHaveBeenCalled();
});

test("キャンセルすると確認を閉じ、実行しない", async () => {
  vi.mocked(api.countMissingThumbnails).mockResolvedValue(3);
  const { user } = await renderPanel();

  await user.click(thumbBulkButton());
  await settle();
  await user.click(cancelButton());

  expect(confirmDialog()).toBeNull();
  expect(api.regenerateMissingThumbnails).not.toHaveBeenCalled();
  expect(api.reprobeMissingMetadata).not.toHaveBeenCalled();
  expect(thumbBulkButton()).toBeEnabled();
});

test("キャンセルのあとにもう一方を確定すると、もう一方だけを実行する", async () => {
  vi.mocked(api.countMissingThumbnails).mockResolvedValue(3);
  vi.mocked(api.countMissingMetadata).mockResolvedValue(5);
  vi.mocked(api.reprobeMissingMetadata).mockResolvedValue(5);
  const { user } = await renderPanel();

  await user.click(thumbBulkButton());
  await settle();
  await user.click(cancelButton());
  await user.click(metaBulkButton());
  await settle();

  expect(within(confirmDialog()!).getByRole("heading")).toHaveTextContent("メタデータの再取得");
  await user.click(confirmButton());
  await settle();

  expect(api.reprobeMissingMetadata).toHaveBeenCalledTimes(1);
  expect(api.regenerateMissingThumbnails).not.toHaveBeenCalled();
});

// ---- 件数の通知 ----

test("件数は、サムネイル失敗とメタデータ失敗の合計が変わるたびに通知し、変わらなければ通知しない", async () => {
  const { onCountChange, update } = await renderPanel();

  expect(onCountChange.mock.calls).toEqual([[0], [3]]);

  // 内訳が変わっても合計が同じなら呼ばない。
  vi.mocked(api.listGenerationFailures).mockResolvedValue(
    failuresOf([thumbFail("a")], [metaFail("b"), metaFail("c")]),
  );
  await emitCatalogChanged();
  expect(onCountChange.mock.calls).toEqual([[0], [3]]);

  vi.mocked(api.listGenerationFailures).mockResolvedValue(failuresOf([], [metaFail("b")]));
  await emitCatalogChanged();
  expect(onCountChange).toHaveBeenLastCalledWith(1);

  vi.mocked(api.listGenerationFailures).mockResolvedValue(
    failuresOf([thumbFail("x"), thumbFail("y")], [metaFail("z"), metaFail("w")]),
  );
  await update({ refreshKey: 1 });
  expect(onCountChange).toHaveBeenLastCalledWith(4);

  vi.mocked(api.listGenerationFailures).mockResolvedValue(failuresOf());
  await update({ refreshKey: 2 });
  expect(onCountChange.mock.calls).toEqual([[0], [3], [1], [4], [0]]);
});

// ---- 現状の挙動(直さない) ----

// 今の動作を固めている: 保留中の確認は閉じても捨てない。
test("現状: 確認を出したまま閉じて開き直すと、確認がまだ出ている", async () => {
  vi.mocked(api.countMissingThumbnails).mockResolvedValue(6);
  vi.mocked(api.regenerateMissingThumbnails).mockResolvedValue(6);
  const { user, update } = await renderPanel();

  await user.click(thumbBulkButton());
  await settle();
  expect(confirmDialog()).toBeInTheDocument();

  await update({ open: false });
  expect(confirmDialog()).toBeNull();

  await update({ open: true });

  expect(confirmDialog()).toBeInTheDocument();
  expect(within(confirmDialog()!).getByRole("heading")).toHaveTextContent("サムネイルの再作成");
  expect(confirmDialog()!.querySelector(".confirm-dialog-body")).toHaveTextContent(
    "6 件の動画のサムネイルを再作成します。",
  );
  expect(api.regenerateMissingThumbnails).not.toHaveBeenCalled();

  await user.click(confirmButton());
  await settle();

  expect(api.regenerateMissingThumbnails).toHaveBeenCalledTimes(1);
  expect(confirmDialog()).toBeNull();
});

// 今の動作を固めている: エラーを消す経路が無い。
test("現状: エラーは一度出ると、その後の取得が成功しても消えない", async () => {
  vi.mocked(api.listGenerationFailures).mockRejectedValueOnce("first failed");
  const { update } = await renderPanel();
  expect(panelError()).toHaveTextContent("first failed");

  vi.mocked(api.listGenerationFailures).mockResolvedValue(DEFAULT());
  await update({ refreshKey: 1 });

  expect(items()).toHaveLength(3);
  expect(panelError()).toHaveTextContent(/^first failed$/);

  await emitCatalogChanged();

  expect(panelError()).toHaveTextContent(/^first failed$/);
});

test("現状: 後から出たエラーは前のエラーを置き換える", async () => {
  vi.mocked(api.listGenerationFailures).mockRejectedValueOnce("first failed");
  vi.mocked(api.retryThumbnailGeneration).mockRejectedValue("retry failed");
  const { user, update } = await renderPanel();

  vi.mocked(api.listGenerationFailures).mockResolvedValue(DEFAULT());
  await update({ refreshKey: 1 });
  await user.click(retryButton(thumbRow("t1")));
  await settle();

  expect(document.querySelectorAll(".generation-failures-error")).toHaveLength(1);
  expect(panelError()).toHaveTextContent(/^retry failed$/);
});

// ---- 見た目 ----

test("失敗が無いときは空の案内を出し、一覧は出さない", async () => {
  await renderPanel({ failures: failuresOf() });

  expect(screen.getByText("生成に失敗したファイルはありません。")).toBeInTheDocument();
  expect(document.querySelector(".generation-failures-list")).toBeNull();
  // 一括の操作は失敗が無くても出す。
  expect(thumbBulkButton()).toBeEnabled();
  expect(metaBulkButton()).toBeEnabled();
});

test("片方の種類だけ失敗があるときも一覧を出す", async () => {
  await renderPanel({ failures: failuresOf([], [metaFail("m")]) });

  expect(screen.queryByText("生成に失敗したファイルはありません。")).toBeNull();
  expect(listing()).toEqual([`${META_BADGE}/m.mp4`]);
});

test("各行にバッジ・ファイル名・失敗回数・再試行を出す", async () => {
  await renderPanel({
    failures: failuresOf(
      [thumbFail("t", { thumbnail_attempts: 5 })],
      [metaFail("m", { metadata_attempts: 2 })],
    ),
  });

  const t = thumbRow("t");
  expect(t.querySelector(".generation-failure-badge")).toHaveTextContent(/^サムネイル生成失敗$/);
  expect(t.querySelector(".generation-failure-info .generation-failure-name")).toHaveTextContent(
    /^t\.mp4$/,
  );
  expect(t.querySelector(".generation-failure-attempts")).toHaveTextContent(/^失敗回数: 5$/);
  expect(within(t).getAllByRole("button")).toHaveLength(1);
  expect(retryButton(t)).toHaveAttribute("type", "button");

  const m = metaRow("m");
  expect(m.querySelector(".generation-failure-badge")).toHaveTextContent(/^メタデータ取得失敗$/);
  expect(m.querySelector(".generation-failure-attempts")).toHaveTextContent(/^失敗回数: 2$/);
  expect(within(m).getAllByRole("button")).toHaveLength(1);
});

test("パスはディレクトリとファイル名に分けて出し、title に元のパスを持つ", async () => {
  await renderPanel({
    failures: failuresOf(
      [thumbFail("t", { file_path: "D:\\movies\\2024\\clip.mkv", file_name: "clip.mkv" })],
      [metaFail("m", { file_path: "\\\\nas\\share\\x.avi", file_name: "x.avi" })],
    ),
  });

  const t = row(THUMB_BADGE, "clip.mkv").querySelector(".file-path-row")!;
  expect(t).toHaveAttribute("title", "D:\\movies\\2024\\clip.mkv");
  expect(t.querySelector(".file-path-dir")!.textContent).toBe("D:\\movies\\2024\\");
  expect(t.querySelector(".file-path-file")!.textContent).toBe("clip.mkv");

  const m = row(META_BADGE, "x.avi").querySelector(".file-path-row")!;
  expect(m).toHaveAttribute("title", "\\\\nas\\share\\x.avi");
  expect(m.querySelector(".file-path-dir")!.textContent).toBe("\\\\nas\\share\\");
  expect(m.querySelector(".file-path-file")!.textContent).toBe("x.avi");
});

test("サムネイル失敗を先に、それぞれ受け取った順に並べる", async () => {
  await renderPanel({
    failures: failuresOf(
      [thumbFail("tb"), thumbFail("ta")],
      [metaFail("mb"), metaFail("ma"), metaFail("mc")],
    ),
  });

  expect(listing()).toEqual([
    `${THUMB_BADGE}/tb.mp4`,
    `${THUMB_BADGE}/ta.mp4`,
    `${META_BADGE}/mb.mp4`,
    `${META_BADGE}/ma.mp4`,
    `${META_BADGE}/mc.mp4`,
  ]);
});

test("一覧が置き換わっても、行は id ごとの状態を保ち、別の行の再試行中の状態を引き継がない", async () => {
  const retry = deferred<void>();
  vi.mocked(api.retryThumbnailGeneration).mockReturnValue(retry.promise);
  const { user } = await renderPanel();

  await user.click(retryButton(thumbRow("t2")));
  expect(retryButton(thumbRow("t2"))).toBeDisabled();

  // t1 が一覧から消えて t2 が先頭に来ても、押せないのは t2 のまま。
  vi.mocked(api.listGenerationFailures).mockResolvedValue(
    failuresOf([thumbFail("t2"), thumbFail("t3")], [metaFail("m1")]),
  );
  await emitCatalogChanged();

  expect(retryButton(thumbRow("t2"))).toBeDisabled();
  expect(retryButton(thumbRow("t3"))).toBeEnabled();
});

test("一括のボタンの文言", async () => {
  await renderPanel();

  expect(thumbBulkButton()).toHaveTextContent(/^未作成のサムネイルをすべて再作成$/);
  expect(metaBulkButton()).toHaveTextContent(/^メタデータをすべて再取得$/);
  expect(thumbBulkButton()).toHaveAttribute("type", "button");
  expect(metaBulkButton()).toHaveAttribute("type", "button");
  expect(thumbBulkButton().parentElement).toHaveClass("generation-failures-bulk");
});
