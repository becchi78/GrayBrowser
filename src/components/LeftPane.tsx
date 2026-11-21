import { useCallback, useEffect, useRef, useState } from "react";
import { getSidebarSectionsCollapsed, saveSidebarSectionsCollapsed } from "../api";
import { readLengthToken } from "../lib/cssTokens";
import {
  effectiveMaxWidth,
  MIN_WIDTH,
  VIDEO_LIST_MIN_WIDTH_PX,
} from "../lib/sidebarResize";
import {
  DEFAULT_SIDEBAR_SECTIONS_COLLAPSED,
  toggleSection,
  type SidebarSectionId,
} from "../lib/sidebarSections";
import type { LibraryCounts } from "../types";
import { FolderSidebar } from "./FolderSidebar";
import { RatingBar } from "./RatingBar";
import { SidebarSection } from "./SidebarSection";
import { FolderIcon, StarIcon, TagIcon } from "./SidebarIcons";
import { TagBar } from "./TagBar";

// User-resizable pane width (200px〜500px). No persistence across restarts --
// state resets to DEFAULT_WIDTH on every mount. MIN_WIDTH/MAX_WIDTH and the
// effectiveMaxWidth clamp live in src/lib/sidebarResize.ts (JSX非依存、
// 単体テスト可能にするため切り出し済み).
const DEFAULT_WIDTH = 200;

// 一覧の下限幅はテーマごとに違う(CSS の --video-list-min-width)ので、
// CSS から読む。CSS を読めない環境でだけ定数に倒す。読み取りは計測用の要素を
// 足して強制レイアウトを起こすので、ドラッグ中の mousemove ごとには呼ばず、
// 値が変わりうる時点(ドラッグ開始・window の resize・テーマの切り替え)でだけ
// 読み直してキャッシュする。
const readVideoListMinWidth = () =>
  readLengthToken("--video-list-min-width", VIDEO_LIST_MIN_WIDTH_PX);

interface Props {
  /** `null` = "すべて" (no folder filter). */
  folderPath: string | null;
  onFolderPathChange: (folderPath: string | null) => void;
  /** Empty = "ALL" (no rating filter). */
  ratings: number[];
  onRatingsChange: (ratings: number[]) => void;
  tagIds: number[];
  onTagIdsChange: (tagIds: number[]) => void;
  /** Bumped after a scan/folder change so the folder list reloads. */
  refreshKey?: number;
  /** Bumped after the tag-order edit dialog saves so TagBar re-fetches. */
  tagBarRefreshKey?: number;
  /** 全体・フォルダ別・レーティング別の件数。未取得の間は null。 */
  counts: LibraryCounts | null;
}

// Left pane: resizable, vertically scrollable column of the three
// collapsible filter sections (フォルダ / レーティング / タグ).
export function LeftPane({
  folderPath,
  onFolderPathChange,
  ratings,
  onRatingsChange,
  tagIds,
  onTagIdsChange,
  refreshKey,
  tagBarRefreshKey,
  counts,
}: Props) {
  const [width, setWidth] = useState(DEFAULT_WIDTH);
  const [collapsed, setCollapsed] = useState(DEFAULT_SIDEBAR_SECTIONS_COLLAPSED);
  // Mutable drag-session state, not React state -- re-rendering on every
  // mousemove pixel would be wasteful; `width` itself is the one value that
  // needs to trigger a re-render.
  const dragRef = useRef<{ startX: number; startWidth: number } | null>(null);
  const minWidthRef = useRef(VIDEO_LIST_MIN_WIDTH_PX);

  // If the window narrows enough that the current width would force
  // `.video-list` below its own floor (see `effectiveMaxWidth`), clamp back
  // down immediately. Deliberately never grows the pane back up when the
  // window widens again -- only pulls it down toward safety. The floor is
  // theme-dependent, so a theme switch re-reads it and clamps too (a wide
  // pane in a narrow window would otherwise leave the list under its floor
  // until the next resize).
  useEffect(() => {
    function reclamp() {
      minWidthRef.current = readVideoListMinWidth();
      setWidth((current) =>
        Math.min(current, effectiveMaxWidth(window.innerWidth, minWidthRef.current)),
      );
    }
    reclamp();
    window.addEventListener("resize", reclamp);
    const observer = new MutationObserver(reclamp);
    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["data-theme"],
    });
    return () => {
      window.removeEventListener("resize", reclamp);
      observer.disconnect();
    };
  }, []);

  useEffect(() => {
    getSidebarSectionsCollapsed()
      .then(setCollapsed)
      .catch((e) => console.error(e));
  }, []);

  const handleToggle = (id: SidebarSectionId) => {
    const next = toggleSection(collapsed, id);
    // Optimistic: the UI flips immediately; a failed save only costs
    // persistence across restarts, so it is logged rather than reverted.
    setCollapsed(next);
    saveSidebarSectionsCollapsed(next).catch((e) => console.error(e));
  };

  const handleResizeMouseDown = useCallback(
    (e: React.MouseEvent<HTMLDivElement>) => {
      e.preventDefault();
      minWidthRef.current = readVideoListMinWidth();
      dragRef.current = { startX: e.clientX, startWidth: width };

      function handleMouseMove(moveEvent: MouseEvent) {
        const drag = dragRef.current;
        if (!drag) return;
        const proposed = drag.startWidth + (moveEvent.clientX - drag.startX);
        const max = effectiveMaxWidth(window.innerWidth, minWidthRef.current);
        setWidth(Math.min(max, Math.max(MIN_WIDTH, proposed)));
      }

      function handleMouseUp() {
        dragRef.current = null;
        window.removeEventListener("mousemove", handleMouseMove);
        window.removeEventListener("mouseup", handleMouseUp);
      }

      window.addEventListener("mousemove", handleMouseMove);
      window.addEventListener("mouseup", handleMouseUp);
    },
    [width],
  );

  return (
    <div className="left-pane" data-testid="left-pane" style={{ width }}>
      {/* The scroll container is separate from the outer pane so the resize
          handle (absolutely positioned on the pane's edge) stays put while
          the sections scroll. */}
      <nav className="left-pane-scroll" data-testid="left-pane-scroll">
        <SidebarSection
          id="folders"
          icon={<FolderIcon className="sidebar-section-icon sidebar-section-icon--folder" />}
          title="フォルダ"
          collapsed={collapsed.folders}
          onToggle={() => handleToggle("folders")}
        >
          <FolderSidebar
            selected={folderPath}
            onSelect={onFolderPathChange}
            refreshKey={refreshKey}
            counts={counts}
          />
        </SidebarSection>
        <SidebarSection
          id="rating"
          icon={<StarIcon className="sidebar-section-icon sidebar-section-icon--rating" />}
          title="レーティング"
          collapsed={collapsed.rating}
          onToggle={() => handleToggle("rating")}
        >
          <RatingBar value={ratings} onChange={onRatingsChange} counts={counts} />
        </SidebarSection>
        <SidebarSection
          id="tags"
          icon={<TagIcon className="sidebar-section-icon sidebar-section-icon--tag" />}
          title="タグ"
          collapsed={collapsed.tags}
          onToggle={() => handleToggle("tags")}
        >
          <TagBar
            selected={tagIds}
            onChange={onTagIdsChange}
            refreshKey={tagBarRefreshKey}
            showCounts
          />
        </SidebarSection>
      </nav>
      <div
        className="left-pane-resize-handle"
        data-testid="folder-sidebar-resize-handle"
        onMouseDown={handleResizeMouseDown}
      />
    </div>
  );
}
