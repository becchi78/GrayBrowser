// ThumbnailGrid の現状の動作を固定する特性テスト。
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { act, fireEvent, screen, within } from "@testing-library/react";
import type { ComponentProps } from "react";
import * as api from "../api";
import * as events from "../events";
import { UnreachableFoldersContext } from "../reachabilityContext";
import { renderWithProviders } from "../testing/render";
import type { AssignTagResultDto, SortDirection, SortField, VideoDto } from "../types";
import { ThumbnailGrid } from "./ThumbnailGrid";
import type { VideoRow as RealVideoRow } from "./VideoRow";

vi.mock("../api", () => ({
  listVideos: vi.fn(),
  listWatchFolders: vi.fn(),
  unregisterVideos: vi.fn(),
  assignTagToVideos: vi.fn(),
  listAllTags: vi.fn(),
}));

vi.mock("../events", () => ({
  onCatalogChanged: vi.fn(),
}));

type VideoRowProps = ComponentProps<typeof RealVideoRow>;

// 偽の行が描画・マウントのたびに書き込む記録。vi.mock の工場関数から参照するため
// vi.hoisted で先に作る。
const rowLog = vi.hoisted(() => ({
  /** 動画 id ごとのマウント回数。key による作り直しを数える。 */
  mounts: new Map<string, number>(),
  /** 各行が受け取った cache。全行で同じ Map かを確かめる。 */
  caches: [] as unknown[],
}));

// 本物の行はサムネイル・プロパティ・タグの取得まで行い、一覧の検証には重いので、
// 受け取った props を data 属性で見せるだけの偽物に差し替える。クリックの修飾キーの
// 読み方は本物と同じ(⌘ を Ctrl として扱う)。
vi.mock("./VideoRow", async () => {
  const { useEffect } = await import("react");
  function FakeVideoRow(props: VideoRowProps) {
    const { video, onSelect, onContextMenu } = props;
    useEffect(() => {
      rowLog.mounts.set(video.id, (rowLog.mounts.get(video.id) ?? 0) + 1);
      // マウント時の id だけを数える(key が変わらない限り id も変わらない)。
      // eslint-disable-next-line react-hooks/exhaustive-deps
    }, []);
    useEffect(() => {
      rowLog.caches.push(props.cache);
    });
    return (
      <div
        data-testid="video-row"
        data-id={video.id}
        data-thumbnail-version={props.thumbnailVersion}
        data-catalog-version={props.catalogVersion}
        data-tags-version={props.tagsVersion}
        data-selected={String(props.isSelected)}
        data-multi-selected={String(props.isMultiSelected)}
        onClick={(e) => onSelect(video, { ctrl: e.ctrlKey || e.metaKey, shift: e.shiftKey })}
        onContextMenu={(e) => onContextMenu(video, e)}
      >
        <span data-testid="row-label">{video.file_name}</span>
        <input data-testid="row-input" />
        <textarea data-testid="row-textarea" />
        <select data-testid="row-select">
          <option>x</option>
        </select>
        <button type="button" data-testid="row-button">
          ボタン
        </button>
        <a href="#row" data-testid="row-link">
          リンク
        </a>
      </div>
    );
  }
  return { VideoRow: FakeVideoRow };
});

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

function video(id: string): VideoDto {
  return {
    id,
    file_path: `C:\\videos\\${id}.mp4`,
    file_name: `${id}.mp4`,
    file_size: 1024,
    duration: 60,
    quick_hash: "qh",
    full_hash: null,
    status: "online",
    rating: 0,
    created_at: "2026-01-01T00:00:00Z",
    thumbnail_ready: true,
    thumbnail_failed: false,
  };
}

const videos = (...ids: string[]) => ids.map(video);
/** 仮想スクロールで一部の行だけが描かれる件数の一覧。 */
const manyVideos = (n: number) => Array.from({ length: n }, (_, i) => video(`v${i}`));

// ---- 仮想スクロールを jsdom で動かすための差し替え ----
//
// jsdom は要素の大きさをすべて 0 と答え、Element#scrollTo も持たない。
// @tanstack/virtual-core は、スクロール要素の offsetWidth/offsetHeight(ResizeObserver
// が無い環境では最初の1回だけ読む)で表示領域の大きさを、scroll イベント時の
// scrollTop で位置を、scrollHeight - clientHeight で最大位置を知り、移動には
// scrollTo({ top }) を使う。一覧のスクロール要素(data-testid="video-list")に限り、
// これらを実際のブラウザのように答えさせる。

const ROW_HEIGHT = 226;
// 1行より低くして、末尾合わせ(align: "end")と先頭合わせの位置が最後の行でも
// 違う値になるようにする。
const VIEWPORT_HEIGHT = 150;
const VIEWPORT_WIDTH = 1000;

const isVideoList = (el: unknown): el is HTMLElement =>
  el instanceof HTMLElement && el.dataset.testid === "video-list";

/** 仮想スクロールの中身の高さ(= 行の高さの合計)。 */
function contentHeight(el: HTMLElement): number {
  const inner = el.firstElementChild;
  return inner instanceof HTMLElement ? parseFloat(inner.style.height) || 0 : 0;
}

const patched: Array<{ proto: object; name: string; original: PropertyDescriptor | undefined }> =
  [];

function patch(proto: object, name: string, descriptor: PropertyDescriptor) {
  patched.push({ proto, name, original: Object.getOwnPropertyDescriptor(proto, name) });
  Object.defineProperty(proto, name, { configurable: true, ...descriptor });
}

/** 一覧の要素だけ fake の値を返し、それ以外は jsdom の本来の値を返す getter。 */
function patchGetter(proto: object, name: string, value: (el: HTMLElement) => number) {
  const original = Object.getOwnPropertyDescriptor(proto, name);
  if (!original?.get) throw new Error(`${name} の getter が見つからない`);
  const originalGet = original.get;
  patch(proto, name, {
    get(this: Element) {
      return isVideoList(this) ? value(this) : originalGet.call(this);
    },
  });
}

/** 一覧の scrollTo が呼ばれた引数。 */
let scrollToCalls: ScrollToOptions[] = [];

function installViewport() {
  scrollToCalls = [];
  patchGetter(HTMLElement.prototype, "offsetHeight", () => VIEWPORT_HEIGHT);
  patchGetter(HTMLElement.prototype, "offsetWidth", () => VIEWPORT_WIDTH);
  patchGetter(Element.prototype, "clientHeight", () => VIEWPORT_HEIGHT);
  patchGetter(Element.prototype, "scrollHeight", (el) =>
    Math.max(contentHeight(el), VIEWPORT_HEIGHT),
  );
  patch(Element.prototype, "scrollTo", {
    writable: true,
    value(this: Element, options?: ScrollToOptions) {
      if (!isVideoList(this)) return;
      scrollToCalls.push({ ...options });
      const max = this.scrollHeight - this.clientHeight;
      const top = Math.max(0, Math.min(options?.top ?? this.scrollTop, max));
      if (top === this.scrollTop) return;
      this.scrollTop = top;
      // ブラウザと同じく、位置が変わったときだけ、後から scroll を発火する。
      // 呼び出し元(レイアウト effect の中など)で同期的に再描画させないため。
      queueMicrotask(() => this.dispatchEvent(new Event("scroll")));
    },
  });
}

function uninstallViewport() {
  for (const { proto, name, original } of patched.reverse()) {
    if (original) Object.defineProperty(proto, name, original);
    else delete (proto as Record<string, unknown>)[name];
  }
  patched.length = 0;
}

// ---- 購読の偽物 ----

let catalogListeners: Array<() => void> = [];
let unlistenCount = 0;

beforeEach(() => {
  // 失敗の経路では console.error に出すだけなので、テストの出力を汚さないよう黙らせる。
  vi.spyOn(console, "error").mockImplementation(() => {});
  installViewport();
  rowLog.mounts.clear();
  rowLog.caches.length = 0;
  catalogListeners = [];
  unlistenCount = 0;
  vi.mocked(events.onCatalogChanged).mockImplementation((cb) => {
    catalogListeners.push(cb);
    return () => {
      unlistenCount += 1;
      catalogListeners = catalogListeners.filter((l) => l !== cb);
    };
  });
  vi.mocked(api.listVideos).mockResolvedValue(videos("a", "b", "c"));
  vi.mocked(api.listWatchFolders).mockResolvedValue(["C:\\videos"]);
  vi.mocked(api.unregisterVideos).mockResolvedValue(0);
  vi.mocked(api.listAllTags).mockResolvedValue([]);
  vi.mocked(api.assignTagToVideos).mockResolvedValue({} as AssignTagResultDto);
});

afterEach(() => {
  vi.useRealTimers();
  uninstallViewport();
});

/** 連なった Promise の解決と、それに続く再描画を落ち着かせる。 */
async function settle() {
  for (let i = 0; i < 3; i++) {
    await act(async () => {});
  }
}

/** 仮想スクロールの位置合わせ(requestAnimationFrame で続く)が終わるのを待つ。 */
async function waitFrames() {
  await act(() => new Promise((resolve) => setTimeout(resolve, 50)));
  await settle();
}

async function emitCatalogChanged() {
  await act(async () => {
    for (const listener of catalogListeners) listener();
  });
  await settle();
}

/** 偽のタイマーを進め、その間に始まった取得の応答を反映させる。 */
async function advance(ms: number) {
  await act(async () => {
    vi.advanceTimersByTime(ms);
  });
  await settle();
}

function useFakeClock() {
  // 仮想スクロールが使う requestAnimationFrame / performance は本物のまま残す。
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "Date"] });
}

interface GridProps {
  refreshKey: number;
  thumbnailVersions: Readonly<Record<string, number>>;
  search: string;
  sortField: SortField;
  sortDirection: SortDirection;
  tagIds: number[];
  folderPath: string | null;
  ratings: number[];
  infoCollapsed: boolean;
  jumpRequest: { percent: number; seq: number } | null;
}

// 配列の props は同一性で比べられるので、変えないときは同じ配列を渡し続ける。
const NO_TAGS: number[] = [];
const NO_RATINGS: number[] = [];
const NO_VERSIONS: Readonly<Record<string, number>> = {};

const BASE: GridProps = {
  refreshKey: 0,
  thumbnailVersions: NO_VERSIONS,
  search: "",
  sortField: "created_at",
  sortDirection: "desc",
  tagIds: NO_TAGS,
  folderPath: null,
  ratings: NO_RATINGS,
  infoCollapsed: false,
  jumpRequest: null,
};

async function renderGrid(overrides: Partial<GridProps> = {}) {
  const onCountChange = vi.fn();
  const onSelectionCountChange = vi.fn();
  let current: GridProps = { ...BASE, ...overrides };
  const ui = (p: GridProps) => (
    <ThumbnailGrid
      {...p}
      onCountChange={onCountChange}
      onSelectionCountChange={onSelectionCountChange}
    />
  );
  const view = renderWithProviders(ui(current));
  await settle();
  // rerender は根の要素を差し替えるので、初回描画と同じ Provider で包まないと
  // 一覧が作り直されて状態が消え、同じ一覧での props の変化を確かめられない。
  const update = async (next: Partial<GridProps>) => {
    current = { ...current, ...next };
    view.rerender(
      <UnreachableFoldersContext.Provider value={[]}>{ui(current)}</UnreachableFoldersContext.Provider>,
    );
    await settle();
  };
  return { ...view, onCountChange, onSelectionCountChange, update };
}

const list = () => screen.getByTestId("video-list");
const rows = () => screen.queryAllByTestId("video-row");
const rowIds = () => rows().map((r) => r.dataset.id);
const row = (id: string) => {
  const found = rows().find((r) => r.dataset.id === id);
  if (!found) throw new Error(`行が見つからない: ${id}`);
  return found;
};
const selectedIds = () => rows().filter((r) => r.dataset.selected === "true").map((r) => r.dataset.id);
const multiSelectedIds = () =>
  rows().filter((r) => r.dataset.multiSelected === "true").map((r) => r.dataset.id);
const catalogVersions = () => rows().map((r) => r.dataset.catalogVersion);
const tagsVersions = () => rows().map((r) => r.dataset.tagsVersion);
const lastListArgs = () => vi.mocked(api.listVideos).mock.lastCall?.[0];
const menu = () => screen.queryByTestId("context-menu");
const menuLabels = () =>
  within(screen.getByTestId("context-menu"))
    .getAllByRole("menuitem")
    .map((b) => b.textContent);
const menuItem = (name: RegExp) => within(screen.getByTestId("context-menu")).getByRole("menuitem", { name });
const confirmDialog = () => screen.queryByTestId("confirm-dialog");
const confirmButton = () => screen.getByTestId("confirm-dialog-confirm-btn");
const cancelButton = () => screen.getByTestId("confirm-dialog-cancel-btn");
const confirmError = () => screen.queryByTestId("confirm-dialog-error");
const emptyHint = () => screen.queryByTestId("video-list-empty-hint");
const emptyView = () => document.querySelector(".video-list-empty");

async function openMenu(id: string, at = { clientX: 10, clientY: 20 }) {
  const notPrevented = fireEvent.contextMenu(row(id), at);
  await settle();
  return notPrevented;
}

async function userScrollTo(top: number) {
  const el = list();
  await act(async () => {
    el.scrollTop = top;
    el.dispatchEvent(new Event("scroll"));
  });
  await settle();
}

// ---- 一覧の取得 ----

test("マウント時に今の条件で一覧を取得し、行を描く", async () => {
  await renderGrid({
    search: "cat",
    sortField: "file_name",
    sortDirection: "asc",
    tagIds: [1, 2],
    folderPath: "C:\\videos",
    ratings: [4, 5],
  });

  expect(api.listVideos).toHaveBeenCalledTimes(1);
  expect(api.listVideos).toHaveBeenCalledWith({
    search: "cat",
    sortField: "file_name",
    sortDirection: "asc",
    tagIds: [1, 2],
    folderPath: "C:\\videos",
    ratings: [4, 5],
  });
  expect(rowIds()).toEqual(["a", "b", "c"]);
  expect(catalogVersions()).toEqual(["1", "1", "1"]);
});

test.each<[string, Partial<GridProps>]>([
  ["refreshKey", { refreshKey: 1 }],
  ["search", { search: "dog" }],
  ["sortField", { sortField: "rating" }],
  ["sortDirection", { sortDirection: "asc" }],
  ["tagIds", { tagIds: [7] }],
  ["folderPath", { folderPath: "D:\\movies" }],
  ["ratings", { ratings: [3] }],
])("%s が変わると、新しい条件で取り直す", async (_name, change) => {
  const { update } = await renderGrid();
  vi.mocked(api.listVideos).mockResolvedValue(videos("x", "y"));

  await update(change);

  expect(api.listVideos).toHaveBeenCalledTimes(2);
  const expected = { ...BASE, ...change };
  expect(lastListArgs()).toEqual({
    search: expected.search,
    sortField: expected.sortField,
    sortDirection: expected.sortDirection,
    tagIds: expected.tagIds,
    folderPath: expected.folderPath,
    ratings: expected.ratings,
  });
  expect(rowIds()).toEqual(["x", "y"]);
  expect(catalogVersions()).toEqual(["2", "2"]);
});

test("条件以外の props が変わっても取り直さない", async () => {
  const { update } = await renderGrid();

  await update({ infoCollapsed: true });
  await update({ thumbnailVersions: { a: 1 } });
  await update({ jumpRequest: { percent: 50, seq: 1 } });

  expect(api.listVideos).toHaveBeenCalledTimes(1);
});

test("catalog:changed の最初のイベントで即座に取り直し、catalogVersion が増える", async () => {
  useFakeClock();
  await renderGrid();
  vi.mocked(api.listVideos).mockResolvedValue(videos("a", "b"));

  await emitCatalogChanged();

  expect(api.listVideos).toHaveBeenCalledTimes(2);
  expect(rowIds()).toEqual(["a", "b"]);
  expect(catalogVersions()).toEqual(["2", "2"]);
});

test("300ms 以内に続いたイベントは1回にまとめ、最初の取得から 300ms で取り直す", async () => {
  useFakeClock();
  await renderGrid();

  await emitCatalogChanged();
  expect(api.listVideos).toHaveBeenCalledTimes(2);

  await advance(100);
  await emitCatalogChanged();
  await advance(100);
  await emitCatalogChanged();
  expect(api.listVideos).toHaveBeenCalledTimes(2);

  await advance(99);
  expect(api.listVideos).toHaveBeenCalledTimes(2);

  await advance(1);
  expect(api.listVideos).toHaveBeenCalledTimes(3);
  expect(catalogVersions()).toEqual(["3", "3", "3"]);

  await advance(1000);
  expect(api.listVideos).toHaveBeenCalledTimes(3);
});

test("まとめた取得の後のイベントも、その取得から 300ms 経つまで待つ", async () => {
  useFakeClock();
  await renderGrid();

  await emitCatalogChanged();
  await advance(100);
  await emitCatalogChanged();
  await advance(200);
  expect(api.listVideos).toHaveBeenCalledTimes(3);

  await advance(50);
  await emitCatalogChanged();
  expect(api.listVideos).toHaveBeenCalledTimes(3);

  await advance(249);
  expect(api.listVideos).toHaveBeenCalledTimes(3);
  await advance(1);
  expect(api.listVideos).toHaveBeenCalledTimes(4);
});

test("300ms 経ってからのイベントは、待たずに取り直す", async () => {
  useFakeClock();
  await renderGrid();

  await emitCatalogChanged();
  await advance(300);
  await emitCatalogChanged();

  expect(api.listVideos).toHaveBeenCalledTimes(3);
});

test("期限前にアンマウントすると取り直さず、購読も解除する", async () => {
  useFakeClock();
  const { unmount } = await renderGrid();

  await emitCatalogChanged();
  await advance(100);
  await emitCatalogChanged();
  await advance(100);
  await emitCatalogChanged();
  expect(unlistenCount).toBe(0);

  unmount();
  expect(unlistenCount).toBe(1);
  expect(catalogListeners).toHaveLength(0);

  await act(async () => {
    vi.advanceTimersByTime(1000);
  });
  expect(api.listVideos).toHaveBeenCalledTimes(2);
});

test("期限前に条件が変わると、古い条件では取り直さず、購読をし直す", async () => {
  useFakeClock();
  const { update } = await renderGrid();

  await emitCatalogChanged();
  await advance(100);
  await emitCatalogChanged();
  await advance(100);
  await emitCatalogChanged();

  await update({ search: "dog" });
  expect(api.listVideos).toHaveBeenCalledTimes(3);
  expect(lastListArgs()?.search).toBe("dog");
  expect(unlistenCount).toBe(1);
  expect(events.onCatalogChanged).toHaveBeenCalledTimes(2);
  expect(catalogListeners).toHaveLength(1);

  await advance(1000);
  expect(api.listVideos).toHaveBeenCalledTimes(3);

  // 新しい購読は新しい条件で取り直す。
  await emitCatalogChanged();
  expect(api.listVideos).toHaveBeenCalledTimes(4);
  expect(lastListArgs()?.search).toBe("dog");
});

test("応答の順序が入れ替わったら、最新の要求の応答だけを反映する", async () => {
  const first = deferred<VideoDto[]>();
  const second = deferred<VideoDto[]>();
  vi.mocked(api.listVideos).mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
  await renderGrid();
  await emitCatalogChanged();

  await act(async () => second.resolve(videos("new")));
  await settle();
  await act(async () => first.resolve(videos("old")));
  await settle();

  expect(rowIds()).toEqual(["new"]);
  expect(catalogVersions()).toEqual(["1"]);
});

test("条件が変わった後に届いた古い条件の応答は捨てる", async () => {
  const first = deferred<VideoDto[]>();
  const second = deferred<VideoDto[]>();
  vi.mocked(api.listVideos).mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
  const { update } = await renderGrid();
  await update({ search: "dog" });

  await act(async () => second.resolve(videos("dog")));
  await settle();
  await act(async () => first.resolve(videos("stale")));
  await settle();

  expect(rowIds()).toEqual(["dog"]);
  expect(catalogVersions()).toEqual(["1"]);
});

test("取り直しに失敗しても、一覧と catalogVersion は変わらない", async () => {
  await renderGrid();
  vi.mocked(api.listVideos).mockRejectedValueOnce(new Error("boom"));

  await emitCatalogChanged();

  expect(api.listVideos).toHaveBeenCalledTimes(2);
  expect(rowIds()).toEqual(["a", "b", "c"]);
  expect(catalogVersions()).toEqual(["1", "1", "1"]);
});

// ---- 登録フォルダ ----

test("登録フォルダの件数が分かるまでは案内を出さず、0件と確定したら空の一覧に案内を出す", async () => {
  const folders = deferred<string[]>();
  vi.mocked(api.listWatchFolders).mockReturnValueOnce(folders.promise);
  vi.mocked(api.listVideos).mockResolvedValue([]);
  await renderGrid();

  expect(emptyView()).toHaveTextContent("動画がまだ登録されていません。");
  expect(emptyHint()).toBeNull();

  await act(async () => folders.resolve([]));
  await settle();

  expect(emptyHint()).toHaveTextContent("「ファイル > フォルダ管理...」からフォルダを登録してください。");
});

test("登録フォルダは refreshKey が変わるたびに取り直し、古い結果は捨てる", async () => {
  const first = deferred<string[]>();
  const second = deferred<string[]>();
  vi.mocked(api.listWatchFolders)
    .mockReturnValueOnce(first.promise)
    .mockReturnValueOnce(second.promise);
  vi.mocked(api.listVideos).mockResolvedValue([]);
  const { update } = await renderGrid();

  await update({ search: "  " });
  expect(api.listWatchFolders).toHaveBeenCalledTimes(1);

  await update({ refreshKey: 1 });
  expect(api.listWatchFolders).toHaveBeenCalledTimes(2);

  await act(async () => second.resolve(["C:\\videos"]));
  await settle();
  await act(async () => first.resolve([]));
  await settle();
  expect(emptyHint()).toBeNull();

  vi.mocked(api.listWatchFolders).mockResolvedValueOnce([]);
  await update({ refreshKey: 2 });
  expect(api.listWatchFolders).toHaveBeenCalledTimes(3);
  expect(emptyHint()).not.toBeNull();
});

// ---- 選択 ----

test("クリック・Ctrl・Shift のクリックで選択が変わり、件数を知らせる", async () => {
  vi.mocked(api.listVideos).mockResolvedValue(videos("a", "b", "c", "d"));
  const { onSelectionCountChange } = await renderGrid();
  expect(selectedIds()).toEqual([]);

  fireEvent.click(row("b"));
  expect(selectedIds()).toEqual(["b"]);
  expect(multiSelectedIds()).toEqual([]);

  fireEvent.click(row("d"), { ctrlKey: true });
  expect(selectedIds()).toEqual(["b", "d"]);
  expect(multiSelectedIds()).toEqual(["b", "d"]);

  // Ctrl でのクリックは起点も動かす(d が起点になる)。
  fireEvent.click(row("a"), { shiftKey: true });
  expect(selectedIds()).toEqual(["a", "b", "c", "d"]);

  fireEvent.click(row("b"), { ctrlKey: true });
  expect(selectedIds()).toEqual(["a", "c", "d"]);

  fireEvent.click(row("c"));
  expect(selectedIds()).toEqual(["c"]);
  expect(multiSelectedIds()).toEqual([]);

  expect(onSelectionCountChange.mock.calls).toEqual([[0], [1], [2], [4], [3], [1]]);
});

test("⌘ のクリックは Ctrl と同じく1件ずつ切り替える", async () => {
  await renderGrid();

  fireEvent.click(row("a"));
  fireEvent.click(row("c"), { metaKey: true });
  expect(selectedIds()).toEqual(["a", "c"]);

  fireEvent.click(row("a"), { metaKey: true });
  expect(selectedIds()).toEqual(["c"]);
});

test("Ctrl+A / ⌘+A で全件を選び(既定動作は止める)、Esc で選択を外す", async () => {
  const { onSelectionCountChange } = await renderGrid();

  expect(fireEvent.keyDown(list(), { key: "a", ctrlKey: true })).toBe(false);
  expect(selectedIds()).toEqual(["a", "b", "c"]);
  expect(multiSelectedIds()).toEqual(["a", "b", "c"]);

  expect(fireEvent.keyDown(list(), { key: "Escape" })).toBe(true);
  expect(selectedIds()).toEqual([]);

  expect(fireEvent.keyDown(list(), { key: "A", metaKey: true })).toBe(false);
  expect(selectedIds()).toEqual(["a", "b", "c"]);

  // 修飾キーの無い a は何もしない。
  fireEvent.keyDown(list(), { key: "Escape" });
  expect(fireEvent.keyDown(list(), { key: "a" })).toBe(true);
  expect(selectedIds()).toEqual([]);

  expect(onSelectionCountChange.mock.calls).toEqual([[0], [3], [0], [3], [0]]);
});

test.each(["row-input", "row-textarea", "row-select"])(
  "行の中の入力欄(%s)では Ctrl+A も Esc も一覧に効かない",
  async (testId) => {
    await renderGrid();
    fireEvent.click(row("b"));
    const field = within(row("a")).getByTestId(testId);

    expect(fireEvent.keyDown(field, { key: "a", ctrlKey: true })).toBe(true);
    expect(selectedIds()).toEqual(["b"]);

    fireEvent.keyDown(field, { key: "Escape" });
    expect(selectedIds()).toEqual(["b"]);
  },
);

test("取り直しで一覧から消えた行だけが選択から外れる", async () => {
  const { onSelectionCountChange } = await renderGrid();
  fireEvent.keyDown(list(), { key: "a", ctrlKey: true });

  vi.mocked(api.listVideos).mockResolvedValue(videos("a", "c", "z"));
  await emitCatalogChanged();

  expect(selectedIds()).toEqual(["a", "c"]);
  expect(onSelectionCountChange).toHaveBeenLastCalledWith(2);
});

test("消えた行が Shift の起点だったら、起点も外れる", async () => {
  vi.mocked(api.listVideos).mockResolvedValue(videos("a", "b", "c", "d"));
  await renderGrid();
  fireEvent.click(row("b"));

  vi.mocked(api.listVideos).mockResolvedValue(videos("a", "c", "d"));
  await emitCatalogChanged();

  // 起点が無いので、Shift のクリックは通常のクリックと同じになる。
  fireEvent.click(row("d"), { shiftKey: true });
  expect(selectedIds()).toEqual(["d"]);
});

test.each<[string, Partial<GridProps>]>([
  ["search", { search: "dog" }],
  ["sortField", { sortField: "rating" }],
  ["sortDirection", { sortDirection: "asc" }],
  ["tagIds", { tagIds: [7] }],
  ["folderPath", { folderPath: "D:\\movies" }],
  ["ratings", { ratings: [3] }],
])("%s が変わると、同じ行が残っても選択を全部外す", async (_name, change) => {
  const { update, onSelectionCountChange } = await renderGrid();
  fireEvent.keyDown(list(), { key: "a", ctrlKey: true });

  await update(change);

  expect(rowIds()).toEqual(["a", "b", "c"]);
  expect(selectedIds()).toEqual([]);
  expect(onSelectionCountChange).toHaveBeenLastCalledWith(0);
});

test("refreshKey が変わっても選択は残る", async () => {
  const { update, onSelectionCountChange } = await renderGrid();
  fireEvent.click(row("a"));
  fireEvent.click(row("c"), { ctrlKey: true });

  await update({ refreshKey: 1 });

  expect(api.listVideos).toHaveBeenCalledTimes(2);
  expect(selectedIds()).toEqual(["a", "c"]);
  expect(onSelectionCountChange.mock.calls).toEqual([[0], [1], [2]]);
});

test.each<[string, object]>([
  ["Ctrl", { ctrlKey: true }],
  ["⌘", { metaKey: true }],
  ["Shift", { shiftKey: true }],
])("%s を押しての mousedown は既定動作を止め、一覧にフォーカスする", async (_name, mods) => {
  await renderGrid();
  expect(document.activeElement).not.toBe(list());

  expect(fireEvent.mouseDown(within(row("b")).getByTestId("row-label"), mods)).toBe(false);
  expect(document.activeElement).toBe(list());
});

test("修飾キーの無い mousedown は何もしない", async () => {
  await renderGrid();

  expect(fireEvent.mouseDown(within(row("b")).getByTestId("row-label"))).toBe(true);
  expect(document.activeElement).not.toBe(list());
});

test.each(["row-input", "row-textarea", "row-select", "row-button", "row-link"])(
  "入力欄・ボタン・リンク(%s)の上の修飾キー付き mousedown は何もしない",
  async (testId) => {
    await renderGrid();

    expect(fireEvent.mouseDown(within(row("b")).getByTestId(testId), { ctrlKey: true })).toBe(
      true,
    );
    expect(document.activeElement).not.toBe(list());
  },
);

// ---- 右クリックのメニュー ----

test("選択の外の行を右クリックすると、その行だけを選んでメニューを出す", async () => {
  const { onSelectionCountChange } = await renderGrid();
  fireEvent.click(row("a"));
  fireEvent.click(row("b"), { ctrlKey: true });

  expect(await openMenu("c", { clientX: 30, clientY: 40 })).toBe(false);

  expect(selectedIds()).toEqual(["c"]);
  expect(menu()).toHaveStyle({ left: "30px", top: "40px" });
  expect(menuLabels()).toEqual(["タグを設定...", "登録から削除"]);
  expect(onSelectionCountChange).toHaveBeenLastCalledWith(1);
});

test("選択の中の行を右クリックすると、選択はそのままで件数つきのメニューを出す", async () => {
  await renderGrid();
  fireEvent.click(row("a"));
  fireEvent.click(row("b"), { ctrlKey: true });

  expect(await openMenu("b")).toBe(false);

  expect(selectedIds()).toEqual(["a", "b"]);
  expect(menuLabels()).toEqual(["タグを設定...(2 件)", "登録から削除(2 件)"]);
});

test("行の中の入力欄を右クリックしても、メニューを出さず既定動作も止めない", async () => {
  await renderGrid();
  fireEvent.click(row("a"));

  expect(fireEvent.contextMenu(within(row("b")).getByTestId("row-input"))).toBe(true);
  await settle();

  expect(menu()).toBeNull();
  expect(selectedIds()).toEqual(["a"]);
});

test("メニューの対象は開いた時点の選択で固定する", async () => {
  const { user } = await renderGrid();
  await openMenu("a");

  // メニューを開いたまま全選択しても、件数も実行対象も変わらない。
  fireEvent.keyDown(list(), { key: "a", ctrlKey: true });
  await settle();
  expect(selectedIds()).toEqual(["a", "b", "c"]);
  expect(menuLabels()).toEqual(["タグを設定...", "登録から削除"]);

  await user.click(menuItem(/^登録から削除/));
  expect(menu()).toBeNull();
  expect(within(confirmDialog()!).getByText("1 件の動画を登録から削除します。")).toBeInTheDocument();

  await user.click(confirmButton());
  expect(api.unregisterVideos).toHaveBeenCalledWith(["a"]);
});

test("一括付与の対象も、メニューを開いた時点の選択で固定する", async () => {
  const { user } = await renderGrid();
  await openMenu("a");

  fireEvent.keyDown(list(), { key: "a", ctrlKey: true });
  await settle();
  await user.click(menuItem(/^タグを設定/));

  const dialog = screen.getByTestId("bulk-tag-dialog");
  expect(dialog).toHaveAccessibleName("1 件の動画にタグを付ける");
  await user.type(within(dialog).getByTestId("bulk-tag-input"), "猫");
  await user.click(within(dialog).getByTestId("bulk-tag-submit-btn"));
  expect(api.assignTagToVideos).toHaveBeenCalledWith(["a"], "猫");
});

test("「タグを設定...」で一括付与のダイアログが開き、付与が終わると tagsVersion が増える", async () => {
  const { user } = await renderGrid();
  fireEvent.click(row("a"));
  fireEvent.click(row("c"), { ctrlKey: true });
  await openMenu("c");
  expect(tagsVersions()).toEqual(["0", "0", "0"]);

  await user.click(menuItem(/^タグを設定/));
  expect(menu()).toBeNull();
  const dialog = screen.getByTestId("bulk-tag-dialog");
  expect(dialog).toHaveAccessibleName("2 件の動画にタグを付ける");

  await user.type(within(dialog).getByTestId("bulk-tag-input"), "猫");
  await user.click(within(dialog).getByTestId("bulk-tag-submit-btn"));

  expect(api.assignTagToVideos).toHaveBeenCalledWith(["a", "c"], "猫");
  expect(screen.queryByTestId("bulk-tag-dialog")).toBeNull();
  expect(tagsVersions()).toEqual(["1", "1", "1"]);
  // タグの付与は一覧を取り直さない。
  expect(api.listVideos).toHaveBeenCalledTimes(1);
});

test("一括付与を取り消しても tagsVersion は変わらない", async () => {
  const { user } = await renderGrid();
  await openMenu("a");
  await user.click(menuItem(/^タグを設定/));

  await user.click(screen.getByTestId("bulk-tag-cancel-btn"));

  expect(screen.queryByTestId("bulk-tag-dialog")).toBeNull();
  expect(tagsVersions()).toEqual(["0", "0", "0"]);
});

// ---- 登録から削除 ----

test("「登録から削除」で件数と注意書きを添えた確認を出す", async () => {
  const { user } = await renderGrid();
  fireEvent.keyDown(list(), { key: "a", ctrlKey: true });
  await openMenu("b");

  await user.click(menuItem(/^登録から削除/));

  const dialog = confirmDialog()!;
  expect(dialog).toHaveAccessibleName("登録から削除");
  expect(within(dialog).getByText("3 件の動画を登録から削除します。")).toBeInTheDocument();
  expect(
    within(dialog).getByText(
      "元の動画ファイルは削除されません。登録フォルダが監視・再スキャンの対象のままだと、次の更新チェックで再登録される場合があります。",
    ),
  ).toBeInTheDocument();
  expect(confirmButton()).toHaveTextContent("登録から削除する");
  expect(confirmButton()).toHaveClass("confirm-dialog-confirm--danger");
  expect(confirmError()).toBeNull();
  expect(api.unregisterVideos).not.toHaveBeenCalled();
});

test("実行中は「処理中...」で押せず二重に実行せず、成功したら閉じる", async () => {
  const pending = deferred<number>();
  vi.mocked(api.unregisterVideos).mockReturnValueOnce(pending.promise);
  const { user } = await renderGrid();
  fireEvent.click(row("a"));
  fireEvent.click(row("b"), { ctrlKey: true });
  await openMenu("a");
  await user.click(menuItem(/^登録から削除/));

  await user.click(confirmButton());
  expect(confirmButton()).toHaveTextContent("処理中...");
  expect(confirmButton()).toBeDisabled();
  expect(cancelButton()).toBeDisabled();

  fireEvent.click(confirmButton());
  expect(api.unregisterVideos).toHaveBeenCalledTimes(1);
  expect(api.unregisterVideos).toHaveBeenCalledWith(["a", "b"]);

  await act(async () => pending.resolve(2));
  await settle();
  expect(confirmDialog()).toBeNull();
});

test("失敗したら確認の中に理由を出し、やり直すと理由を消す", async () => {
  vi.mocked(api.unregisterVideos).mockRejectedValueOnce("削除できません");
  const { user } = await renderGrid();
  await openMenu("a");
  await user.click(menuItem(/^登録から削除/));

  await user.click(confirmButton());

  const error = within(confirmDialog()!).getByRole("alert");
  expect(error).toBe(confirmError());
  expect(error).toHaveTextContent("削除できません");
  expect(error).toHaveClass("confirm-dialog-error");
  expect(confirmButton()).toHaveTextContent("登録から削除する");

  const retry = deferred<number>();
  vi.mocked(api.unregisterVideos).mockReturnValueOnce(retry.promise);
  await user.click(confirmButton());
  expect(confirmError()).toBeNull();
  expect(api.unregisterVideos).toHaveBeenCalledTimes(2);

  await act(async () => retry.resolve(1));
  await settle();
  expect(confirmDialog()).toBeNull();
});

test("キャンセルで閉じ、理由も消えて、開き直したときに残らない", async () => {
  vi.mocked(api.unregisterVideos).mockRejectedValueOnce("削除できません");
  const { user } = await renderGrid();
  await openMenu("a");
  await user.click(menuItem(/^登録から削除/));
  await user.click(confirmButton());
  expect(confirmError()).not.toBeNull();

  await user.click(cancelButton());
  expect(confirmDialog()).toBeNull();
  expect(api.unregisterVideos).toHaveBeenCalledTimes(1);

  await openMenu("a");
  await user.click(menuItem(/^登録から削除/));
  expect(confirmDialog()).not.toBeNull();
  expect(confirmError()).toBeNull();
});

test("確認を開いたまま別の行から「登録から削除」を選ぶと、前回の理由を消して対象を差し替える", async () => {
  vi.mocked(api.unregisterVideos).mockRejectedValueOnce("削除できません");
  const { user } = await renderGrid();
  await openMenu("a");
  await user.click(menuItem(/^登録から削除/));
  await user.click(confirmButton());
  expect(confirmError()).not.toBeNull();

  fireEvent.click(row("b"));
  fireEvent.click(row("c"), { ctrlKey: true });
  await openMenu("c");
  fireEvent.click(menuItem(/^登録から削除/));
  await settle();

  expect(confirmError()).toBeNull();
  expect(within(confirmDialog()!).getByText("2 件の動画を登録から削除します。")).toBeInTheDocument();
  await user.click(confirmButton());
  expect(api.unregisterVideos).toHaveBeenLastCalledWith(["b", "c"]);
});

test("削除で一覧が空になっても、確認ダイアログは実行中のまま残り、終わると閉じる", async () => {
  vi.mocked(api.listVideos).mockResolvedValue(videos("a"));
  const pending = deferred<number>();
  vi.mocked(api.unregisterVideos).mockReturnValueOnce(pending.promise);
  const { user } = await renderGrid();
  await openMenu("a");
  await user.click(menuItem(/^登録から削除/));
  await user.click(confirmButton());

  vi.mocked(api.listVideos).mockResolvedValue([]);
  await emitCatalogChanged();
  expect(screen.queryByTestId("video-list")).toBeNull();
  expect(emptyView()).not.toBeNull();
  expect(confirmButton()).toHaveTextContent("処理中...");

  await act(async () => pending.resolve(1));
  await settle();
  expect(confirmDialog()).toBeNull();
});

test("空の表示に切り替わった後の失敗も、確認の中に理由を出す", async () => {
  vi.mocked(api.listVideos).mockResolvedValue(videos("a"));
  const pending = deferred<number>();
  vi.mocked(api.unregisterVideos).mockReturnValueOnce(pending.promise);
  const { user } = await renderGrid();
  await openMenu("a");
  await user.click(menuItem(/^登録から削除/));
  await user.click(confirmButton());

  vi.mocked(api.listVideos).mockResolvedValue([]);
  await emitCatalogChanged();
  await act(async () => pending.reject("失敗しました"));
  await settle();

  expect(emptyView()).not.toBeNull();
  expect(confirmError()).toHaveTextContent("失敗しました");

  await user.click(cancelButton());
  expect(confirmDialog()).toBeNull();
});

// ---- スクロールとジャンプ ----

test("search が変わったときだけ先頭に戻し、並べ替えや取り直しでは戻さない", async () => {
  vi.mocked(api.listVideos).mockResolvedValue(manyVideos(100));
  const { update } = await renderGrid();
  await userScrollTo(2000);

  await update({ sortField: "file_name" });
  await update({ sortDirection: "asc" });
  await update({ tagIds: [1] });
  await update({ refreshKey: 1 });
  await emitCatalogChanged();
  expect(list().scrollTop).toBe(2000);

  await update({ search: "v" });
  expect(list().scrollTop).toBe(0);
  expect(scrollToCalls).toContainEqual({ top: 0 });
});

test("ジャンプは jumpTargetIndex の行へ、100% は末尾合わせ・それ以外は先頭合わせで動かす", async () => {
  vi.mocked(api.listVideos).mockResolvedValue(manyVideos(100));
  const { update } = await renderGrid();

  await update({ jumpRequest: { percent: 50, seq: 1 } });
  await waitFrames();
  expect(list().scrollTop).toBe(50 * ROW_HEIGHT);
  expect(rowIds()).toContain("v50");

  await update({ jumpRequest: { percent: 10, seq: 2 } });
  await waitFrames();
  expect(list().scrollTop).toBe(10 * ROW_HEIGHT);

  await update({ jumpRequest: { percent: 100, seq: 3 } });
  await waitFrames();
  // 最後の行の下端が表示領域の下端に揃う位置(= 最大のスクロール位置)。
  expect(list().scrollTop).toBe(100 * ROW_HEIGHT - VIEWPORT_HEIGHT);
  expect(rowIds()).toContain("v99");

  await update({ jumpRequest: { percent: 0, seq: 4 } });
  await waitFrames();
  expect(list().scrollTop).toBe(0);
});

test("割り切れない件数では行番号を切り捨てた行へ動かす", async () => {
  vi.mocked(api.listVideos).mockResolvedValue(manyVideos(15));
  const { update } = await renderGrid();

  await update({ jumpRequest: { percent: 50, seq: 1 } });
  await waitFrames();

  expect(list().scrollTop).toBe(7 * ROW_HEIGHT);
});

test("ジャンプは jumpRequest が変わったときだけで、取り直しや他の props では動かさない", async () => {
  vi.mocked(api.listVideos).mockResolvedValue(manyVideos(100));
  const { update } = await renderGrid();
  const request = { percent: 50, seq: 1 };
  await update({ jumpRequest: request });
  await waitFrames();
  expect(list().scrollTop).toBe(50 * ROW_HEIGHT);

  await userScrollTo(0);
  await waitFrames();
  await update({ jumpRequest: request, sortField: "file_name" });
  await update({ jumpRequest: request, infoCollapsed: true });
  vi.mocked(api.listVideos).mockResolvedValue(manyVideos(90));
  await emitCatalogChanged();
  await waitFrames();
  expect(list().scrollTop).toBe(0);

  // 同じ percent でも seq が変われば、もう一度動かす(行番号は今の件数で決める)。
  await update({ jumpRequest: { percent: 50, seq: 2 } });
  await waitFrames();
  expect(list().scrollTop).toBe(45 * ROW_HEIGHT);
});

test("取り直しの応答とジャンプの要求が同じ描画に入っても、新しい件数で行を決める", async () => {
  vi.mocked(api.listVideos).mockResolvedValue(manyVideos(20));
  const { rerender, onCountChange, onSelectionCountChange } = await renderGrid();
  const refetch = deferred<VideoDto[]>();
  vi.mocked(api.listVideos).mockReturnValueOnce(refetch.promise);
  await emitCatalogChanged();
  expect(api.listVideos).toHaveBeenCalledTimes(2);

  // 応答の反映(setVideos)と props の変化を1つの act にまとめ、同じ描画に入れる。
  await act(async () => {
    refetch.resolve(manyVideos(100));
    await refetch.promise;
    await Promise.resolve();
    rerender(
      <UnreachableFoldersContext.Provider value={[]}>
        <ThumbnailGrid
          {...BASE}
          jumpRequest={{ percent: 50, seq: 1 }}
          onCountChange={onCountChange}
          onSelectionCountChange={onSelectionCountChange}
        />
      </UnreachableFoldersContext.Provider>,
    );
  });
  await settle();
  await waitFrames();

  expect(onCountChange).toHaveBeenLastCalledWith(100);
  expect(list().scrollTop).toBe(50 * ROW_HEIGHT);
});

test("件数を onCountChange に知らせ、件数が同じ取り直しでは知らせない", async () => {
  const { update, onCountChange } = await renderGrid();
  expect(onCountChange.mock.calls).toEqual([[0], [3]]);

  vi.mocked(api.listVideos).mockResolvedValue(videos("x", "y", "z"));
  await emitCatalogChanged();
  expect(onCountChange.mock.calls).toEqual([[0], [3]]);

  vi.mocked(api.listVideos).mockResolvedValue([]);
  await update({ search: "none" });
  expect(onCountChange.mock.calls).toEqual([[0], [3], [0]]);
});

// ---- 空の表示 ----

test("一覧が空で絞り込みも無ければ「まだ登録されていません」を出し、一覧の要素は描かない", async () => {
  vi.mocked(api.listVideos).mockResolvedValue([]);
  await renderGrid();

  expect(emptyView()).toHaveTextContent(/^動画がまだ登録されていません。$/);
  expect(emptyHint()).toBeNull();
  expect(screen.queryByTestId("video-list")).toBeNull();
  expect(rows()).toHaveLength(0);
});

test.each<[string, Partial<GridProps>]>([
  ["search", { search: "dog" }],
  ["前後に空白のある search", { search: "  dog " }],
  ["tagIds", { tagIds: [1] }],
])("%s で絞り込み中に空なら「見つかりません」を出し、案内は出さない", async (_name, filter) => {
  vi.mocked(api.listVideos).mockResolvedValue([]);
  vi.mocked(api.listWatchFolders).mockResolvedValue([]);
  await renderGrid(filter);

  expect(emptyView()).toHaveTextContent(/^該当する動画が見つかりません。$/);
  expect(emptyHint()).toBeNull();
});

test.each<[string, Partial<GridProps>]>([
  ["絞り込みなし", {}],
  ["空白だけの search", { search: "   " }],
  ["folderPath だけ", { folderPath: "C:\\videos" }],
  ["ratings だけ", { ratings: [5] }],
])("%s で空なら「まだ登録されていません」を出し、フォルダが0件なら案内も出す", async (_name, filter) => {
  vi.mocked(api.listVideos).mockResolvedValue([]);
  vi.mocked(api.listWatchFolders).mockResolvedValue([]);
  await renderGrid(filter);

  expect(emptyView()).toHaveTextContent(/^動画がまだ登録されていません。/);
  expect(emptyHint()).not.toBeNull();
  expect(emptyView()).toContainElement(emptyHint());
});

// ---- 見た目 ----

test("一覧の要素は class・data-testid・tabIndex を持ち、情報欄を畳むと class が変わる", async () => {
  const { update } = await renderGrid();

  expect(list()).toHaveAttribute("class", "video-list");
  expect(list().tabIndex).toBe(-1);
  expect(list()).toHaveAttribute("tabindex", "-1");

  await update({ infoCollapsed: true });
  expect(list()).toHaveAttribute("class", "video-list video-list--info-collapsed");

  await update({ infoCollapsed: false });
  expect(list()).toHaveAttribute("class", "video-list");
});

test("仮想スクロールの行は絶対配置で、行の高さと位置を style に持つ", async () => {
  await renderGrid();

  const inner = list().firstElementChild as HTMLElement;
  expect(inner.style.height).toBe(`${3 * ROW_HEIGHT}px`);
  expect(inner.style.position).toBe("relative");
  expect(inner.style.width).toBe("100%");

  const wrappers = Array.from(inner.children) as HTMLElement[];
  expect(wrappers).toHaveLength(3);
  wrappers.forEach((wrapper, i) => {
    expect(wrapper.style.position).toBe("absolute");
    expect(wrapper.style.top).toBe("0px");
    expect(wrapper.style.left).toBe("0px");
    expect(wrapper.style.width).toBe("100%");
    expect(wrapper.style.height).toBe(`${ROW_HEIGHT}px`);
    expect(wrapper.style.transform).toBe(`translateY(${i * ROW_HEIGHT}px)`);
    expect(wrapper.firstElementChild).toBe(rows()[i]);
  });
});

test("表示領域の外の行は描かない", async () => {
  vi.mocked(api.listVideos).mockResolvedValue(manyVideos(100));
  await renderGrid();

  // 表示領域(1行ぶん)+ 前後5行の先読み。
  expect(rowIds()).toEqual(["v0", "v1", "v2", "v3", "v4", "v5"]);

  await userScrollTo(20 * ROW_HEIGHT);
  expect(rowIds()).toEqual([
    "v15",
    "v16",
    "v17",
    "v18",
    "v19",
    "v20",
    "v21",
    "v22",
    "v23",
    "v24",
    "v25",
  ]);
});

test("thumbnailVersion を行へ渡し、変わった行だけを作り直す", async () => {
  const { update } = await renderGrid({ thumbnailVersions: { b: 2 } });
  expect(rows().map((r) => r.dataset.thumbnailVersion)).toEqual(["0", "2", "0"]);
  expect(Object.fromEntries(rowLog.mounts)).toEqual({ a: 1, b: 1, c: 1 });

  await update({ thumbnailVersions: { a: 1, b: 2 } });

  expect(rows().map((r) => r.dataset.thumbnailVersion)).toEqual(["1", "2", "0"]);
  expect(Object.fromEntries(rowLog.mounts)).toEqual({ a: 2, b: 1, c: 1 });

  // 他の props の変化や取り直しでは作り直さない。
  await update({ infoCollapsed: true });
  await emitCatalogChanged();
  fireEvent.click(row("a"));
  expect(Object.fromEntries(rowLog.mounts)).toEqual({ a: 2, b: 1, c: 1 });
});

test("全行が同じサムネイルのキャッシュ(Map)を受け取り、描き直しても同じものを使う", async () => {
  const { update } = await renderGrid();
  await update({ thumbnailVersions: { a: 1 } });
  await emitCatalogChanged();

  expect(rowLog.caches.length).toBeGreaterThan(3);
  const [first] = rowLog.caches;
  expect(first).toBeInstanceOf(Map);
  expect(rowLog.caches.every((c) => c === first)).toBe(true);
});
