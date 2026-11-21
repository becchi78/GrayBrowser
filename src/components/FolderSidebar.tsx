import { checkFolderReachability } from "../api";
import type { LibraryCounts } from "../types";
import { useIsFolderUnreachable } from "../reachabilityContext";
import { useWatchFolderList } from "../useWatchFolderList";
import { CountPill } from "./CountPill";

interface Props {
  /** `null` = "すべて" (no folder filter). */
  selected: string | null;
  onSelect: (folderPath: string | null) => void;
  /**
   * Bumped by the parent after a scan/folder-add/folder-remove completes so
   * this list picks up watch folder changes without a full remount. Folder
   * management itself lives in `FolderDialog` -- this component only
   * mirrors `list_watch_folders` for the selection UI.
   */
  refreshKey?: number;
  /** 件数のピル。省略・未取得の間は出さない。 */
  counts?: LibraryCounts | null;
}

// Registered-folder list + selection state. "すべて" clears the filter;
// clicking a folder scopes the grid to it via `list_videos`'s `folder_path`
// argument (wired end-to-end through ThumbnailGrid -> api.listVideos).
export function FolderSidebar({ selected, onSelect, refreshKey, counts }: Props) {
  const folders = useWatchFolderList(refreshKey) ?? [];
  const folderCounts = new Map(counts?.folders.map((f) => [f.folder_path, f.count]));

  return (
    <div className="folder-sidebar" data-testid="folder-sidebar">
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
        <CountPill count={counts?.total} />
      </button>
      {folders.map((folder) => (
        <FolderSidebarItem
          key={folder}
          folder={folder}
          active={selected === folder}
          onSelect={onSelect}
          count={folderCounts.get(folder)}
        />
      ))}
    </div>
  );
}

interface ItemProps {
  folder: string;
  active: boolean;
  onSelect: (folderPath: string | null) => void;
  count: number | undefined;
}

// A separate component because the reachability hook cannot be called inside
// the parent's map(). The badge comes before the path so that it stays
// visible when a long path is ellipsized at the end.
function FolderSidebarItem({ folder, active, onSelect, count }: ItemProps) {
  const unreachable = useIsFolderUnreachable(folder);
  return (
    <button
      type="button"
      className={
        active ? "folder-sidebar-item folder-sidebar-item--active" : "folder-sidebar-item"
      }
      data-testid="folder-sidebar-item"
      title={unreachable ? "到達できません(オフライン)" : folder}
      onClick={() => {
        onSelect(folder);
        // Not awaited: the answer arrives as a reachability event, and a
        // stalled share must not delay the folder switch. Clicking the
        // already selected folder checks again on purpose.
        checkFolderReachability(folder).catch(console.error);
      }}
    >
      {unreachable && (
        <span className="folder-sidebar-badge" role="img" aria-label="到達できません">
          ⚠
        </span>
      )}
      {folder}
      <CountPill count={count} />
    </button>
  );
}
