import { useVirtualizer } from "@tanstack/react-virtual";
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type MouseEvent,
  type ReactNode,
} from "react";
import { readLengthToken } from "../lib/cssTokens";
import { formatCount } from "../lib/filterChips";
import { isTextEntryTarget } from "../lib/textEntryTarget";
import type { SortDirection, SortField, VideoDto } from "../types";
import { useHasNoWatchFolders } from "../useHasNoWatchFolders";
import { useJumpToPercent } from "../useJumpToPercent";
import { useListSelection } from "../useListSelection";
import { useUnregisterVideos } from "../useUnregisterVideos";
import { useVideoList } from "../useVideoList";
import { VideoListOverlays, type VideoListMenu } from "./VideoListOverlays";
import { VideoRow } from "./VideoRow";

// 行の高さの情報源は CSS の `--row-height`(テーマごとに違う)。ここの値は、
// CSS を読めない環境(jsdom のテスト)でだけ使う既定値で、従来の実測値
// (サムネイル6枚 + 情報パネル + 行の余白)。
const ROW_HEIGHT_FALLBACK = 226;

interface Props {
  refreshKey: number;
  /** Per-video count of manual thumbnail replacements; a change re-reads that row's images. */
  thumbnailVersions: Readonly<Record<string, number>>;
  search: string;
  sortField: SortField;
  sortDirection: SortDirection;
  tagIds: number[];
  /** `null` = "すべて" (no folder filter); see FolderSidebar. */
  folderPath: string | null;
  /** Empty = "すべて表示" (no rating filter); see RatingBar. */
  ratings: number[];
  /** 現在のテーマ。変わると行の高さを CSS から読み直し、表示位置を保って再計測する。 */
  theme?: string;
  /** 絞り込みなしの総数(一覧上の件数ヘッダー用)。未取得の間は null / 省略。 */
  totalCount?: number | null;
  /** Hides every row's right-hand info column (CSS only; rows still mount it). */
  infoCollapsed: boolean;
  /** `seq` lets pressing the same percent twice in a row still count as a new request. */
  jumpRequest: { percent: number; seq: number } | null;
  /** Reports the current (filtered/sorted) row count, for the jump button. */
  onCountChange: (count: number) => void;
  /** Reports how many rows are currently selected, for the status bar. */
  onSelectionCountChange: (count: number) => void;
}

export function ThumbnailGrid({
  refreshKey,
  thumbnailVersions,
  search,
  sortField,
  sortDirection,
  tagIds,
  folderPath,
  ratings,
  infoCollapsed,
  theme,
  totalCount = null,
  jumpRequest,
  onCountChange,
  onSelectionCountChange,
}: Props) {
  const query = { search, sortField, sortDirection, tagIds, folderPath, ratings };
  const parentRef = useRef<HTMLDivElement>(null);
  const thumbnailCache = useRef(new Map<string, string[]>());

  const noWatchFolders = useHasNoWatchFolders(refreshKey);
  // 選択は一覧を受け取るので、一覧の後に作る。onLoaded は取得の応答が届いてから
  // (描画の後に)呼ばれるので、その時点で selection は作られている。
  const { videos, catalogVersion } = useVideoList(query, refreshKey, (rows) =>
    selection.prune(rows),
  );
  const selection = useListSelection(videos, onSelectionCountChange, parentRef);
  const clearSelection = selection.clear;

  // A selection must never silently apply to rows the user can no longer
  // see, so any change of what the list shows clears it. refreshKey is left
  // out on purpose: it re-fetches the same view, which prune handles.
  // clearSelection は描画をまたいで同じ関数なので、依存に入れても条件の変化の
  // ときだけ動く。
  //
  // この effect は effect の中で同期的に state を更新するので、React Compiler の
  // 解析対象では react-hooks/set-state-in-effect のエラーになる(選択のフックの
  // 中に置くと、useCallback 越しの呼び出しでもエラーになる)。ここで通るのは、
  // このコンポーネント全体が useVirtualizer のため解析対象外
  // (react-hooks/incompatible-library)だからで、useVirtualizer をフックへ
  // 移すとこのコンポーネントも検査対象になり、この effect の書き方を
  // 見直す必要がある。
  useEffect(() => {
    clearSelection();
  }, [clearSelection, search, sortField, sortDirection, tagIds, folderPath, ratings]);

  const unregister = useUnregisterVideos();
  // 右クリックで開くメニューの位置と対象。対象はメニューを開いた時点の選択で
  // 固定する(メニューを開いている間に選択が変わっても、見せた件数と実行する
  // 対象がずれないようにする)。
  const [menu, setMenu] = useState<VideoListMenu | null>(null);
  // 一括付与の対象(開いた時点で固定)。null のときダイアログは閉じている。
  const [bulkTagIds, setBulkTagIds] = useState<string[] | null>(null);
  // 一括付与で各行のタグが変わったことを行へ伝える。catalogVersion は生成進捗の
  // たびに増えるので、タグの再取得には使わない。
  const [tagsVersion, setTagsVersion] = useState(0);

  // Resets scroll to the top only when the *search query* changes -- not on
  // sort-order change (still viewing the same result set, just reordered)
  // or catalog:changed refetches. A new search starting fresh at the top
  // avoids the virtualizer's scroll offset pointing past a now-shorter
  // filtered result.
  useEffect(() => {
    parentRef.current?.scrollTo({ top: 0 });
  }, [search]);

  // One video = one row, so the virtualizer counts rows 1:1 against `videos`.
  // Any overflow from the row's own 6 fixed-width thumbnails is handled purely
  // by `.video-row-thumbnails`'s own `overflow-x: auto` in CSS.
  //
  // 行の高さは CSS から読む。見積もりのたびに DOM を読むと数千行で重いので、
  // 読んだ値を保持し、テーマが変わったときだけ読み直す。
  const rowHeightRef = useRef<number | null>(null);
  // 先頭に見えている行。テーマの切り替えで行の高さが変わっても同じ行を
  // 先頭に戻すために、切り替え前の値を持っておく。
  const firstIndexRef = useRef(0);
  // 行の高さを最後に読んだときのテーマ。
  const settledThemeRef = useRef(theme);
  const restoreIndexRef = useRef<number | null>(null);
  const rowVirtualizer = useVirtualizer({
    count: videos.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => (rowHeightRef.current ??= readLengthToken("--row-height", ROW_HEIGHT_FALLBACK)),
    overscan: 5,
    onChange: (instance) => {
      // <html> の属性は切り替えの瞬間に変わり、このコンポーネントが気づくのは
      // その後の描画。その間に来る通知(レイアウトの変化による再計算)で、
      // 新しい行の高さのまま古いスクロール位置から求めた行番号を記録しないよう、
      // 属性と読み直し済みのテーマが一致している間だけ記録する。
      if (document.documentElement.dataset.theme !== settledThemeRef.current) return;
      firstIndexRef.current = instance.range?.startIndex ?? 0;
    },
  });

  // テーマが変わったら行の高さを読み直して再計測し、切り替え前の先頭行へ戻す。
  // 戻すのは、新しい合計の高さで描画し直した後(下の useLayoutEffect)。
  // 古い合計の高さのままだと、行が高くなるテーマで末尾付近の位置が頭打ちになる。
  useEffect(() => {
    if (settledThemeRef.current === theme) return;
    settledThemeRef.current = theme;
    rowHeightRef.current = null;
    restoreIndexRef.current = firstIndexRef.current;
    rowVirtualizer.measure();
  }, [theme, rowVirtualizer]);

  useLayoutEffect(() => {
    const index = restoreIndexRef.current;
    if (index === null) return;
    restoreIndexRef.current = null;
    rowVirtualizer.scrollToIndex(index, { align: "start" });
  });

  useEffect(() => {
    onCountChange(videos.length);
  }, [videos.length, onCountChange]);

  useJumpToPercent(jumpRequest, videos.length, rowVirtualizer.scrollToIndex);

  function handleContextMenu(video: VideoDto, e: MouseEvent) {
    // 行内の入力欄(タグ入力など)は行メニューを出さず、document 側の
    // 入力欄用メニュー(または抑止)に任せる。
    if (isTextEntryTarget(e.target)) return;
    e.preventDefault();
    const ids = selection.selectForContextMenu(video.id);
    setMenu({ x: e.clientX, y: e.clientY, ids });
  }

  const closeMenu = useCallback(() => setMenu(null), []);

  const cache = thumbnailCache.current;
  const virtualRows = rowVirtualizer.getVirtualItems();

  // 削除で一覧が空になっても、確認ダイアログの後始末が途切れないよう、
  // メニューと確認ダイアログは一覧の空表示と共通で描画する。
  const overlays = (
    <VideoListOverlays
      menu={menu}
      videos={videos}
      onCloseMenu={closeMenu}
      onRequestBulkTag={setBulkTagIds}
      bulkTagIds={bulkTagIds}
      onTagged={() => setTagsVersion((v) => v + 1)}
      onCloseBulkTag={() => setBulkTagIds(null)}
      onRequestUnregister={unregister.request}
      pendingUnregister={unregister.pending}
      unregisterBusy={unregister.busy}
      unregisterError={unregister.error}
      onConfirmUnregister={unregister.run}
      onCancelUnregister={unregister.cancel}
    />
  );

  if (videos.length === 0) {
    const filtered = search.trim() !== "" || tagIds.length > 0;
    return (
      <>
        <ListLayer matched={0} totalCount={totalCount}>
          <div className="video-list-empty">
          {filtered ? "該当する動画が見つかりません。" : "動画がまだ登録されていません。"}
          {!filtered && noWatchFolders && (
            <div data-testid="video-list-empty-hint">
              メニュー(≡)の「フォルダ管理...」からフォルダを登録してください。
            </div>
          )}
          </div>
        </ListLayer>
        {overlays}
      </>
    );
  }

  return (
    <>
      <ListLayer matched={videos.length} totalCount={totalCount}>
      <div
        ref={parentRef}
        className={infoCollapsed ? "video-list video-list--info-collapsed" : "video-list"}
        data-testid="video-list"
        tabIndex={-1}
        onKeyDown={selection.handleKeyDown}
        onMouseDown={selection.handleMouseDown}
      >
        <div
          style={{
            height: rowVirtualizer.getTotalSize(),
            position: "relative",
            width: "100%",
          }}
        >
          {virtualRows.map((virtualRow) => {
            const video = videos[virtualRow.index];
            return (
              <div
                key={virtualRow.key}
                style={{
                  position: "absolute",
                  top: 0,
                  left: 0,
                  width: "100%",
                  height: virtualRow.size,
                  transform: `translateY(${virtualRow.start}px)`,
                }}
              >
                <VideoRow
                  key={`${video.id}:${thumbnailVersions[video.id] ?? 0}`}
                  thumbnailVersion={thumbnailVersions[video.id] ?? 0}
                  video={video}
                  cache={cache}
                  catalogVersion={catalogVersion}
                  tagsVersion={tagsVersion}
                  isSelected={selection.selectedIds.has(video.id)}
                  isMultiSelected={selection.multiSelected && selection.selectedIds.has(video.id)}
                  onSelect={selection.handleSelect}
                  onContextMenu={handleContextMenu}
                />
              </div>
            );
          })}
        </div>
      </div>
      </ListLayer>
      {overlays}
    </>
  );
}

// 一覧を包む器。先頭に件数のヘッダーを持ち、フルーエントのテーマではレイヤー面
// (左上だけ角丸の面)、コンソールのテーマではカードの面になる。見た目は CSS の
// 出し分けで、構造は全テーマで同じ(テーマごとに React の木を分けない)。
// ヘッダーは行の仮想スクロールの外側(.video-list の兄弟)なので、行の高さには影響しない。
function ListLayer({
  matched,
  totalCount,
  children,
}: {
  matched: number;
  totalCount: number | null;
  children: ReactNode;
}) {
  return (
    <div className="list-layer" data-testid="list-layer">
      <div className="list-header" data-testid="list-header">
        <span className="list-header-title">LIBRARY</span>
        <strong className="list-header-count">{formatCount(matched)}</strong>
        <span className="list-header-unit">件</span>
        {totalCount !== null && (
          <span className="list-header-total">/ 全 {formatCount(totalCount)} 件</span>
        )}
      </div>
      {children}
    </div>
  );
}
