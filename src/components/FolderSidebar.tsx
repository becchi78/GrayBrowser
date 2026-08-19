import { useCallback, useEffect, useRef, useState } from "react";
import { listWatchFolders } from "../api";
import { effectiveMaxWidth, MIN_WIDTH } from "../lib/sidebarResize";

// User-resizable sidebar width, replacing the previous fixed 200px
// (App.css's `.folder-sidebar` no longer sets `width` at all -- this
// component now owns it as state, applied via inline `style`). No
// persistence across app restarts -- state resets to DEFAULT_WIDTH on
// every mount.
// MIN_WIDTH/MAX_WIDTH and the effectiveMaxWidth clamp calculation itself
// live in src/lib/sidebarResize.ts (JSX非依存、単体テスト可能にするため
// 切り出し済み -- src/lib/sidebarResize.test.ts参照).
const DEFAULT_WIDTH = 200;

interface Props {
  /** `null` = "すべて" (no folder filter). */
  selected: string | null;
  onSelect: (folderPath: string | null) => void;
  /**
   * Bumped by the parent after a scan/folder-add/folder-remove completes so
   * this list picks up watch folder changes without a full remount. Folder
   * management itself lives in `FolderDialog` -- this component only
   * mirrors `list_watch_folders` for the sidebar's selection UI, plus
   * (below) the entry point that opens that dialog.
   */
  refreshKey?: number;
  /** Opens `FolderDialog`. */
  onOpenFolderDialog: () => void;
}

// Registered-folder list + selection state for the always-visible sidebar
// (user-resizable, 200px〜500px -- see this file's own width-state
// comments above). "すべて" clears the filter; clicking a folder scopes
// the grid to it via `list_videos`'s `folder_path` argument (wired
// end-to-end through ThumbnailGrid -> api.listVideos).
export function FolderSidebar({ selected, onSelect, refreshKey, onOpenFolderDialog }: Props) {
  const [folders, setFolders] = useState<string[]>([]);
  const [width, setWidth] = useState(DEFAULT_WIDTH);
  // Mutable drag-session state, not React state -- read/written only inside
  // the mousedown/mousemove/mouseup handlers below, never rendered directly
  // (re-rendering on every mousemove pixel would be wasteful; `width`
  // itself, the one value that *does* need to trigger a re-render, is
  // already a separate `useState` above).
  const dragRef = useRef<{ startX: number; startWidth: number } | null>(null);

  // If the window is narrowed enough (e.g. dragged
  // smaller, or a smaller monitor) that the current width would now force
  // `.video-list` below its own floor (see `effectiveMaxWidth`'s own
  // comment), clamp back down immediately rather than leaving the sidebar
  // sitting at a width that's already in the danger zone until the next
  // drag happens to fix it. Deliberately does NOT grow the sidebar back up
  // if the window widens again afterward -- only ever pulls it down toward
  // safety, never changes it unprompted otherwise.
  useEffect(() => {
    function handleResize() {
      setWidth((current) => Math.min(current, effectiveMaxWidth(window.innerWidth)));
    }
    window.addEventListener("resize", handleResize);
    return () => window.removeEventListener("resize", handleResize);
  }, []);

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

  useEffect(() => {
    let cancelled = false;
    listWatchFolders()
      .then((rows) => {
        if (!cancelled) setFolders(rows);
      })
      .catch(console.error);
    return () => {
      cancelled = true;
    };
  }, [refreshKey]);

  return (
    <nav className="folder-sidebar" data-testid="folder-sidebar" style={{ width }}>
      <button
        type="button"
        className={
          selected === null
            ? "folder-sidebar-item folder-sidebar-item--active"
            : "folder-sidebar-item"
        }
        data-testid="folder-sidebar-all"
        onClick={() => onSelect(null)}
      >
        すべて
      </button>
      {folders.map((folder) => (
        <button
          key={folder}
          type="button"
          className={
            selected === folder
              ? "folder-sidebar-item folder-sidebar-item--active"
              : "folder-sidebar-item"
          }
          data-testid="folder-sidebar-item"
          title={folder}
          onClick={() => onSelect(folder)}
        >
          {folder}
        </button>
      ))}
      <button
        type="button"
        className="folder-sidebar-manage-link"
        data-testid="folder-sidebar-manage-link"
        onClick={onOpenFolderDialog}
      >
        フォルダ管理 ▸
      </button>
      {/* The drag handle -- see App.css's
          `.folder-sidebar-resize-handle` for the absolute positioning that
          overlays it along the sidebar's full-height right edge. Rendered
          last so it paints on top of the folder buttons above without
          needing an explicit z-index (later DOM order already wins ties). */}
      <div
        className="folder-sidebar-resize-handle"
        data-testid="folder-sidebar-resize-handle"
        onMouseDown={handleResizeMouseDown}
      />
    </nav>
  );
}
