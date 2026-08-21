import { useVirtualizer } from "@tanstack/react-virtual";
import { useEffect, useRef, useState } from "react";
import { listVideos } from "../api";
import { onCatalogChanged } from "../events";
import type { SortDirection, SortField, VideoDto } from "../types";
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
  /** `null` = "すべて表示" (no rating filter); see RatingBar. */
  minRating: number | null;
}

export function ThumbnailGrid({
  refreshKey,
  search,
  sortField,
  sortDirection,
  tagIds,
  folderPath,
  minRating,
}: Props) {
  const [videos, setVideos] = useState<VideoDto[]>([]);
  // Highlights the selected row -- the detail panel this used to open is
  // gone, so selection no longer has any other effect than the
  // `.video-row--selected` class VideoRow applies itself.
  const [selectedVideoId, setSelectedVideoId] = useState<string | null>(null);
  const parentRef = useRef<HTMLDivElement>(null);
  const thumbnailCache = useRef(new Map<string, string[]>());

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

    const tick = async () => {
      try {
        const rows = await listVideos({
          search,
          sortField,
          sortDirection,
          tagIds,
          folderPath,
          minRating,
        });
        if (cancelled) return;
        setVideos(rows);
        // Clears the selection highlight if the selected video disappeared
        // from the list entirely (e.g. its watch folder was removed) --
        // otherwise `.video-row--selected` would just never match again.
        setSelectedVideoId((prev) =>
          prev && rows.some((v) => v.id === prev) ? prev : null,
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
    // folderPath/minRating via closure, so those need to be dependencies too
    // (not just refreshKey).
  }, [refreshKey, search, sortField, sortDirection, tagIds, folderPath, minRating]);

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

  const cache = thumbnailCache.current;
  const virtualRows = rowVirtualizer.getVirtualItems();

  if (videos.length === 0) {
    const filtered = search.trim() !== "" || tagIds.length > 0;
    return (
      <div className="video-list-empty">
        {filtered ? "該当する動画が見つかりません。" : "動画がまだ登録されていません。"}
      </div>
    );
  }

  return (
    <div ref={parentRef} className="video-list" data-testid="video-list">
      <div style={{ height: rowVirtualizer.getTotalSize(), position: "relative", width: "100%" }}>
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
                isSelected={video.id === selectedVideoId}
                onSelect={(v) => setSelectedVideoId(v.id)}
              />
            </div>
          );
        })}
      </div>
    </div>
  );
}
