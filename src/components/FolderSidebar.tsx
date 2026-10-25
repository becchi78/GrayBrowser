import { useEffect, useState } from "react";
import { checkFolderReachability, listWatchFolders } from "../api";

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
}

// Registered-folder list + selection state. "すべて" clears the filter;
// clicking a folder scopes the grid to it via `list_videos`'s `folder_path`
// argument (wired end-to-end through ThumbnailGrid -> api.listVideos).
export function FolderSidebar({ selected, onSelect, refreshKey }: Props) {
  const [folders, setFolders] = useState<string[]>([]);

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
          onClick={() => {
            onSelect(folder);
            // Not awaited: the answer arrives as a reachability event, and a
            // stalled share must not delay the folder switch. Clicking the
            // already selected folder checks again on purpose.
            checkFolderReachability(folder).catch(console.error);
          }}
        >
          {folder}
        </button>
      ))}
    </div>
  );
}
