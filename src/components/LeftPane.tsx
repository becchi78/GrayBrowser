import { useCallback, useEffect, useRef, useState } from "react";
import { getSidebarSectionsCollapsed, saveSidebarSectionsCollapsed } from "../api";
import { effectiveMaxWidth, MIN_WIDTH } from "../lib/sidebarResize";
import {
  DEFAULT_SIDEBAR_SECTIONS_COLLAPSED,
  toggleSection,
  type SidebarSectionId,
} from "../lib/sidebarSections";
import { FolderSidebar } from "./FolderSidebar";
import { RatingBar } from "./RatingBar";
import { SidebarSection } from "./SidebarSection";
import { TagBar } from "./TagBar";

// User-resizable pane width (200px〜500px). No persistence across restarts --
// state resets to DEFAULT_WIDTH on every mount. MIN_WIDTH/MAX_WIDTH and the
// effectiveMaxWidth clamp live in src/lib/sidebarResize.ts (JSX非依存、
// 単体テスト可能にするため切り出し済み).
const DEFAULT_WIDTH = 200;

interface Props {
  /** `null` = "すべて" (no folder filter). */
  folderPath: string | null;
  onFolderPathChange: (folderPath: string | null) => void;
  /** `null` = "ALL" (no rating filter). */
  rating: number | null;
  onRatingChange: (rating: number | null) => void;
  tagIds: number[];
  onTagIdsChange: (tagIds: number[]) => void;
  /** Bumped after a scan/folder change so the folder list reloads. */
  refreshKey?: number;
  /** Bumped after the pinned-tag edit dialog saves so TagBar re-fetches. */
  tagBarRefreshKey?: number;
}

// Left pane: resizable, vertically scrollable column of the three
// collapsible filter sections (フォルダ / 評価 / タグ).
export function LeftPane({
  folderPath,
  onFolderPathChange,
  rating,
  onRatingChange,
  tagIds,
  onTagIdsChange,
  refreshKey,
  tagBarRefreshKey,
}: Props) {
  const [width, setWidth] = useState(DEFAULT_WIDTH);
  const [collapsed, setCollapsed] = useState(DEFAULT_SIDEBAR_SECTIONS_COLLAPSED);
  // Mutable drag-session state, not React state -- re-rendering on every
  // mousemove pixel would be wasteful; `width` itself is the one value that
  // needs to trigger a re-render.
  const dragRef = useRef<{ startX: number; startWidth: number } | null>(null);

  // If the window narrows enough that the current width would force
  // `.video-list` below its own floor (see `effectiveMaxWidth`), clamp back
  // down immediately. Deliberately never grows the pane back up when the
  // window widens again -- only pulls it down toward safety.
  useEffect(() => {
    function handleResize() {
      setWidth((current) => Math.min(current, effectiveMaxWidth(window.innerWidth)));
    }
    window.addEventListener("resize", handleResize);
    return () => window.removeEventListener("resize", handleResize);
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
      dragRef.current = { startX: e.clientX, startWidth: width };

      function handleMouseMove(moveEvent: MouseEvent) {
        const drag = dragRef.current;
        if (!drag) return;
        const proposed = drag.startWidth + (moveEvent.clientX - drag.startX);
        const max = effectiveMaxWidth(window.innerWidth);
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
          title="フォルダ"
          collapsed={collapsed.folders}
          onToggle={() => handleToggle("folders")}
        >
          <FolderSidebar
            selected={folderPath}
            onSelect={onFolderPathChange}
            refreshKey={refreshKey}
          />
        </SidebarSection>
        <SidebarSection
          id="rating"
          title="評価"
          collapsed={collapsed.rating}
          onToggle={() => handleToggle("rating")}
        >
          <RatingBar value={rating} onChange={onRatingChange} />
        </SidebarSection>
        <SidebarSection
          id="tags"
          title="タグ"
          collapsed={collapsed.tags}
          onToggle={() => handleToggle("tags")}
        >
          <TagBar selected={tagIds} onChange={onTagIdsChange} refreshKey={tagBarRefreshKey} />
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
