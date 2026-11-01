import { useVirtualizer } from "@tanstack/react-virtual";
import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type KeyboardEvent,
  type MouseEvent,
} from "react";
import { listVideos, listWatchFolders, unregisterVideos } from "../api";
import { onCatalogChanged } from "../events";
import { jumpTargetIndex } from "../lib/jumpToPercent";
import { createLatestOnlyGuard } from "../lib/latestRequest";
import { isTextEntryTarget } from "../lib/textEntryTarget";
import {
  clearSelection,
  clickSelect,
  pruneSelection,
  selectAll,
  selectionForContextMenu,
  type ClickModifiers,
  type SelectionState,
} from "../lib/multiSelect";
import type { SortDirection, SortField, VideoDto } from "../types";
import { ConfirmDialog } from "./ConfirmDialog";
import { BulkTagDialog } from "./BulkTagDialog";
import { ContextMenu } from "./ContextMenu";
import { VideoRow } from "./VideoRow";

// A row hosts 6 thumbnails + metadata/rating/tag-editor instead of a
// single thumbnail cell, so the old 180px grid-cell height no longer
// fits. Measured via CDP (WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=
// --remote-debugging-port) against real `.video-row-info` content at the
// "one tag assigned" baseline: 205.890625px + `.video-row`'s own padding
// (0.5em top+bottom = 16px) + ~4px margin = 226px. Rows with 2+ tags that
// wrap onto a second line fall back to `.video-row-info`'s own
// `overflow-y: auto` rather than growing ROW_HEIGHT further. The fixed
// 150px thumbnail height (+16px padding = 166px) comfortably fits within
// 226px too.
const ROW_HEIGHT = 226;
// Max re-fetch frequency while catalog:changed events are arriving.
const NOTIFY_THROTTLE_MS = 300;

interface Props {
  refreshKey: number;
  search: string;
  sortField: SortField;
  sortDirection: SortDirection;
  tagIds: number[];
  /** `null` = "すべて" (no folder filter); see FolderSidebar. */
  folderPath: string | null;
  /** Empty = "すべて表示" (no rating filter); see RatingBar. */
  ratings: number[];
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
  search,
  sortField,
  sortDirection,
  tagIds,
  folderPath,
  ratings,
  infoCollapsed,
  jumpRequest,
  onCountChange,
  onSelectionCountChange,
}: Props) {
  const [videos, setVideos] = useState<VideoDto[]>([]);
  // 一覧を再取得するたびに増える。VideoDtoには詳細情報の取得状況が含まれず、
  // 行は自分で状況を知る手段がないため、「一覧が更新された」ことだけを行に
  // 伝えて、取得が終わっていない行に取り直させる。
  const [catalogVersion, setCatalogVersion] = useState(0);
  // Starts false so the "register a folder" hint never flashes before the
  // first fetch resolves; only a confirmed empty list shows it.
  const [noWatchFolders, setNoWatchFolders] = useState(false);
  // Highlights the selected rows -- the detail panel selection used to open
  // is gone, so selection has no other effect than the CSS classes VideoRow
  // applies itself. Held by id so rows scrolled out of the virtualizer's
  // window stay selected.
  const [selection, setSelection] = useState<SelectionState>(clearSelection);
  // 右クリックで開くメニューの位置と対象。対象はメニューを開いた時点の選択で
  // 固定する(メニューを開いている間に選択が変わっても、見せた件数と実行する
  // 対象がずれないようにする)。
  const [menu, setMenu] = useState<{
    x: number;
    y: number;
    ids: string[];
  } | null>(null);
  const [pendingUnregister, setPendingUnregister] = useState<string[] | null>(null);
  const [unregisterBusy, setUnregisterBusy] = useState(false);
  const [unregisterError, setUnregisterError] = useState<string | null>(null);
  // 一括付与の対象(開いた時点で固定)。null のときダイアログは閉じている。
  const [bulkTagIds, setBulkTagIds] = useState<string[] | null>(null);
  // 一括付与で各行のタグが変わったことを行へ伝える。catalogVersion は生成進捗の
  // たびに増えるので、タグの再取得には使わない。
  const [tagsVersion, setTagsVersion] = useState(0);
  const parentRef = useRef<HTMLDivElement>(null);
  const thumbnailCache = useRef(new Map<string, string[]>());

  // 登録フォルダが0件かどうか。空の一覧に、フォルダの登録先を案内するために使う。
  useEffect(() => {
    let cancelled = false;
    listWatchFolders()
      .then((rows) => {
        if (!cancelled) setNoWatchFolders(rows.length === 0);
      })
      .catch(console.error);
    return () => {
      cancelled = true;
    };
  }, [refreshKey]);

  // Re-fetches once on mount/refreshKey change, then again on every
  // catalog:changed event from the backend -- replaces the old finite
  // POLL_INTERVAL_MS setTimeout loop (no more idle polling once every
  // video's thumbnail is ready, since there's no timer left to stop).
  //
  // A burst of events (many files added at once, or many thumbnails
  // finishing in quick succession) is throttled to at most one re-fetch per
  // NOTIFY_THROTTLE_MS -- but as a *throttle*, not a trailing-edge debounce:
  // the first event in a burst fires immediately (leading edge), and while
  // events keep arriving faster than the throttle window, a re-fetch still
  // happens roughly every NOTIFY_THROTTLE_MS rather than only once after the
  // whole burst goes quiet. That's what lets the grid fill in progressively
  // during a large batch instead of staying frozen until everything finishes.
  useEffect(() => {
    let cancelled = false;
    // 一覧の再取得は応答の順序が入れ替わりうる。古い要求の応答（まだ
    // 生成中の状態）が後から届いて新しい一覧を上書きしないよう、最新の
    // 要求の応答だけを反映する。依存が変わって effect が再実行されたときの
    // 古い応答は、上の cancelled で破棄されるので、ガードは effect ごとに
    // 作れば足りる。
    const guard = createLatestOnlyGuard();

    const tick = async () => {
      const requestId = guard.begin();
      try {
        const rows = await listVideos({
          search,
          sortField,
          sortDirection,
          tagIds,
          folderPath,
          ratings,
        });
        if (cancelled || !guard.isLatest(requestId)) return;
        setVideos(rows);
        setCatalogVersion((v) => v + 1);
        // Only videos that vanished from the list (e.g. a watch folder was
        // removed) are dropped; a plain refresh must not wipe the user's
        // selection.
        setSelection((prev) =>
          pruneSelection(
            prev,
            rows.map((v) => v.id),
          ),
        );
      } catch (e) {
        console.error(e);
      }
    };
    tick();

    let lastRun = 0;
    let trailingTimer: ReturnType<typeof setTimeout> | null = null;
    let trailingPending = false;

    const scheduleTick = () => {
      const elapsed = Date.now() - lastRun;
      if (elapsed >= NOTIFY_THROTTLE_MS) {
        lastRun = Date.now();
        trailingPending = false;
        tick();
        return;
      }
      trailingPending = true;
      if (trailingTimer === null) {
        trailingTimer = setTimeout(() => {
          trailingTimer = null;
          if (trailingPending) {
            trailingPending = false;
            lastRun = Date.now();
            tick();
          }
        }, NOTIFY_THROTTLE_MS - elapsed);
      }
    };

    const unlisten = onCatalogChanged(scheduleTick);

    return () => {
      cancelled = true;
      unlisten();
      if (trailingTimer !== null) clearTimeout(trailingTimer);
    };
    // catalog:changed re-fetches always use the latest search/sort/tagIds/
    // folderPath/ratings via closure, so those need to be dependencies too
    // (not just refreshKey).
  }, [refreshKey, search, sortField, sortDirection, tagIds, folderPath, ratings]);

  // A selection must never silently apply to rows the user can no longer
  // see, so any change of what the list shows clears it. refreshKey is left
  // out on purpose: it re-fetches the same view, which prune handles.
  useEffect(() => {
    setSelection(clearSelection());
  }, [search, sortField, sortDirection, tagIds, folderPath, ratings]);

  useEffect(() => {
    onSelectionCountChange(selection.ids.size);
  }, [selection.ids.size, onSelectionCountChange]);

  // Resets scroll to the top only when the *search query* changes -- not on
  // sort-order change (still viewing the same result set, just reordered)
  // or catalog:changed refetches. A new search starting fresh at the top
  // avoids the virtualizer's scroll offset pointing past a now-shorter
  // filtered result.
  useEffect(() => {
    parentRef.current?.scrollTo({ top: 0 });
  }, [search]);

  // One video = one row now, so the virtualizer counts rows 1:1 against
  // `videos` -- no column-count calculation (the
  // old ResizeObserver-driven `columns` state) is needed anymore. Any
  // overflow from the row's own 6 fixed-width thumbnails is handled purely
  // by `.video-row-thumbnails`'s own `overflow-x: auto` in CSS.
  const rowVirtualizer = useVirtualizer({
    count: videos.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => ROW_HEIGHT,
    overscan: 5,
  });

  useEffect(() => {
    onCountChange(videos.length);
  }, [videos.length, onCountChange]);

  // Keyed on the request object only: depending on `videos` would yank the
  // user back to the jump target whenever the list refetches, so the latest
  // length is read through a ref instead.
  const videoCountRef = useRef(videos.length);
  videoCountRef.current = videos.length;
  const scrollToIndex = rowVirtualizer.scrollToIndex;
  const scrollToIndexRef = useRef(scrollToIndex);
  scrollToIndexRef.current = scrollToIndex;
  useEffect(() => {
    if (jumpRequest === null) return;
    const index = jumpTargetIndex(videoCountRef.current, jumpRequest.percent);
    if (index === null) return;
    scrollToIndexRef.current(index, {
      align: jumpRequest.percent === 100 ? "end" : "start",
    });
  }, [jumpRequest]);

  function handleSelect(video: VideoDto, mods: ClickModifiers) {
    const orderedIds = videos.map((v) => v.id);
    setSelection((prev) => clickSelect(prev, orderedIds, video.id, mods));
  }

  function handleContextMenu(video: VideoDto, e: MouseEvent) {
    // 行内の入力欄(タグ入力など)では、標準の切り取り/貼り付けメニューを残す。
    if (isTextEntryTarget(e.target)) return;
    e.preventDefault();
    const next = selectionForContextMenu(selection, video.id);
    setSelection(next);
    setMenu({ x: e.clientX, y: e.clientY, ids: [...next.ids] });
  }

  const closeMenu = useCallback(() => setMenu(null), []);

  function cancelUnregister() {
    setPendingUnregister(null);
    setUnregisterError(null);
  }

  async function runUnregister() {
    if (pendingUnregister === null || unregisterBusy) return;
    setUnregisterBusy(true);
    setUnregisterError(null);
    try {
      await unregisterVideos(pendingUnregister);
      setPendingUnregister(null);
    } catch (e) {
      console.error(e);
      setUnregisterError(String(e));
    } finally {
      setUnregisterBusy(false);
    }
  }

  function handleKeyDown(e: KeyboardEvent<HTMLDivElement>) {
    if (isTextEntryTarget(e.target)) return;
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "a") {
      e.preventDefault();
      setSelection(selectAll(videos.map((v) => v.id)));
    } else if (e.key === "Escape") {
      setSelection(clearSelection());
    }
  }

  // Shift/Ctrl+click would otherwise start the browser's own text selection.
  // preventDefault also suppresses the focus change, and the keyboard
  // shortcuts need the list focused, so focus it by hand.
  function handleMouseDown(e: MouseEvent<HTMLDivElement>) {
    if (!(e.ctrlKey || e.metaKey || e.shiftKey)) return;
    if (e.target instanceof Element && e.target.closest("input, textarea, select, button, a")) {
      return;
    }
    e.preventDefault();
    parentRef.current?.focus({ preventScroll: true });
  }

  const multiSelected = selection.ids.size >= 2;
  const cache = thumbnailCache.current;
  const virtualRows = rowVirtualizer.getVirtualItems();

  // 削除で一覧が空になっても、確認ダイアログの後始末が途切れないよう、
  // メニューと確認ダイアログは一覧の空表示と共通で描画する。
  const overlays = (
    <>
      {menu && (
        <ContextMenu
          position={{ x: menu.x, y: menu.y }}
          items={[
            {
              label: menu.ids.length >= 2 ? `タグを設定...(${menu.ids.length} 件)` : "タグを設定...",
              onSelect: () => setBulkTagIds(menu.ids),
            },
            {
              label: menu.ids.length >= 2 ? `登録から削除(${menu.ids.length} 件)` : "登録から削除",
              danger: true,
              onSelect: () => {
                setUnregisterError(null);
                setPendingUnregister(menu.ids);
              },
            },
          ]}
          onClose={closeMenu}
        />
      )}
      <BulkTagDialog
        videoIds={bulkTagIds}
        onTagged={() => setTagsVersion((v) => v + 1)}
        onClose={() => setBulkTagIds(null)}
      />
      <ConfirmDialog
        open={pendingUnregister !== null}
        title="登録から削除"
        confirmLabel="登録から削除する"
        danger
        busy={unregisterBusy}
        onConfirm={runUnregister}
        onCancel={cancelUnregister}
      >
        <p>{pendingUnregister?.length ?? 0} 件の動画を登録から削除します。</p>
        <p>
          元の動画ファイルは削除されません。登録フォルダが監視・再スキャンの対象のままだと、次の更新チェックで再登録される場合があります。
        </p>
        {unregisterError && (
          <p className="confirm-dialog-error" role="alert" data-testid="confirm-dialog-error">
            {unregisterError}
          </p>
        )}
      </ConfirmDialog>
    </>
  );

  if (videos.length === 0) {
    const filtered = search.trim() !== "" || tagIds.length > 0;
    return (
      <>
        <div className="video-list-empty">
          {filtered ? "該当する動画が見つかりません。" : "動画がまだ登録されていません。"}
          {!filtered && noWatchFolders && (
            <div data-testid="video-list-empty-hint">
              「ファイル &gt; フォルダ管理...」からフォルダを登録してください。
            </div>
          )}
        </div>
        {overlays}
      </>
    );
  }

  return (
    <>
      <div
        ref={parentRef}
        className={infoCollapsed ? "video-list video-list--info-collapsed" : "video-list"}
        data-testid="video-list"
        tabIndex={-1}
        onKeyDown={handleKeyDown}
        onMouseDown={handleMouseDown}
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
                  key={video.id}
                  video={video}
                  cache={cache}
                  catalogVersion={catalogVersion}
                  tagsVersion={tagsVersion}
                  isSelected={selection.ids.has(video.id)}
                  isMultiSelected={multiSelected && selection.ids.has(video.id)}
                  onSelect={handleSelect}
                  onContextMenu={handleContextMenu}
                />
              </div>
            );
          })}
        </div>
      </div>
      {overlays}
    </>
  );
}
