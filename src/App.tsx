import { useCallback, useEffect, useRef, useState } from "react";
import "./App.css";
import { FfmpegNotice } from "./components/FfmpegNotice";
import { AboutDialog } from "./components/AboutDialog";
import { LogViewerDialog } from "./components/LogViewerDialog";
import { FolderDialog } from "./components/FolderDialog";
import { LeftPane } from "./components/LeftPane";
import { SettingsDialog } from "./components/SettingsDialog";
import { SearchBox } from "./components/SearchBox";
import { SortControl } from "./components/SortControl";
import { StatusBar } from "./components/StatusBar";
import { UpdateDialog } from "./components/UpdateDialog";
import { UpdateNotice } from "./components/UpdateNotice";
import { TagBarEditDialog } from "./components/TagBarEditDialog";
import { ThumbnailGrid } from "./components/ThumbnailGrid";
import { WbImportDialog } from "./components/WbImportDialog";
import {
  getInfoPaneCollapsed,
  loadViewState,
  saveViewState,
  setInfoPaneCollapsed as saveInfoPaneCollapsed,
} from "./api";
import {
  onMenuAbout,
  onMenuCheckUpdate,
  onMenuOpenFolderDialog,
  onMenuOpenLogDialog,
  onMenuOpenSettingsDialog,
  onMenuOpenTagBarDialog,
  onMenuOpenWbImportDialog,
  onMenuStyleSelected,
} from "./events";
import { EditContextMenu, type EditTarget } from "./components/EditContextMenu";
import { contextMenuAction, type MenuPosition } from "./lib/contextMenu";
import { isTextEditTarget } from "./lib/textEntryTarget";
import { INITIAL_VIEW_STATE, fromSaved, isSameViewState, toSaved, type ViewState } from "./lib/viewState";

// Only "default" exists today. New styles (e.g. "grid") get added here as
// additional literal values, plus one entry in the viewStyle -> component
// mapping below -- no other App.tsx changes.
type ViewStyle = "default";

// Wait this long after the last change before saving, so typing in the search
// box does not write on every keystroke.
const VIEW_STATE_SAVE_DELAY_MS = 500;

function App() {
  // The grid is not rendered until the saved state is known: mounting it with
  // the initial state first would run a query for a view the user is about to
  // leave. A failed load just continues with the initial state.
  const [initialView, setInitialView] = useState<ViewState | null>(null);
  useEffect(() => {
    loadViewState()
      .then((saved) => setInitialView(saved === null ? INITIAL_VIEW_STATE : fromSaved(saved)))
      .catch((e) => {
        console.error(e);
        setInitialView(INITIAL_VIEW_STATE);
      });
  }, []);
  if (initialView === null) {
    return <div className="app" data-testid="app-loading" />;
  }
  return <AppContent initialView={initialView} />;
}

function AppContent({ initialView }: { initialView: ViewState }) {
  const [refreshKey, setRefreshKey] = useState(0);
  // 手動の紐付けでサムネイルが置き換わった動画の id -> 回数。サムネイルは
  // 動画ごとに取得済みの画像をメモリに持つため、ファイルだけ差し替わっても
  // 表示は変わらない。この値が変わった行だけ取り直させる。
  const [thumbnailVersions, setThumbnailVersions] = useState<Readonly<Record<string, number>>>({});
  const [search, setSearch] = useState(initialView.search);
  const [sortField, setSortField] = useState(initialView.sortField);
  const [sortDirection, setSortDirection] = useState(initialView.sortDirection);
  const [tagIds, setTagIds] = useState(initialView.tagIds);
  // `null` = "すべて" (no folder filter).
  const [folderPath, setFolderPath] = useState(initialView.folderPath);
  // Empty = "ALL" (no rating filter). Independent filter axis from
  // tagIds -- see RatingBar's own comment.
  const [ratings, setRatings] = useState(initialView.ratings);
  // FolderDialog/WbImportDialog are reached only via the native menu
  // ("ファイル > フォルダ管理.../.wbインポート...").
  const [folderDialogOpen, setFolderDialogOpen] = useState(false);
  const [wbImportDialogOpen, setWbImportDialogOpen] = useState(false);
  // TagBarEditDialog is reached only via the native menu ("タグ > タグの編集...").
  // tagBarRefreshKey is `refreshKey` scoped to just TagBar: bumped when the
  // dialog saves a new tag order or deletes a tag, so TagBar re-fetches
  // without a full remount.
  const [tagBarDialogOpen, setTagBarDialogOpen] = useState(false);
  const [aboutDialogOpen, setAboutDialogOpen] = useState(false);
  const [logDialogOpen, setLogDialogOpen] = useState(false);
  const [settingsDialogOpen, setSettingsDialogOpen] = useState(false);
  const [updateDialogOpen, setUpdateDialogOpen] = useState(false);
  const [tagBarRefreshKey, setTagBarRefreshKey] = useState(0);
  const [viewStyle, setViewStyle] = useState<ViewStyle>("default");
  const [infoPaneCollapsed, setInfoPaneCollapsed] = useState(false);
  const [videoCount, setVideoCount] = useState(0);
  const [selectedCount, setSelectedCount] = useState(0);
  const [editMenu, setEditMenu] = useState<{ target: EditTarget; position: MenuPosition } | null>(
    null,
  );
  const [jumpRequest, setJumpRequest] = useState<{ percent: number; seq: number } | null>(null);

  // What the backend already holds, so an unchanged state is never rewritten.
  const lastSavedRef = useRef(initialView);
  useEffect(() => {
    const current: ViewState = { search, folderPath, sortField, sortDirection, tagIds, ratings };
    if (isSameViewState(current, lastSavedRef.current)) return;
    const timer = setTimeout(() => {
      lastSavedRef.current = current;
      // A failed save only costs restoring this state next launch, so it is
      // logged rather than surfaced.
      saveViewState(toSaved(current)).catch((e) => console.error(e));
    }, VIEW_STATE_SAVE_DELAY_MS);
    return () => clearTimeout(timer);
  }, [search, folderPath, sortField, sortDirection, tagIds, ratings]);

  useEffect(() => {
    getInfoPaneCollapsed()
      .then(setInfoPaneCollapsed)
      .catch((e) => console.error(e));
  }, []);

  useEffect(() => {
    // Registered on document (bubble), so per-row custom menus handled by
    // React's root listener run first and are unaffected; a second
    // preventDefault is harmless. Selection and copy are not touched.
    const handler = (e: MouseEvent) => {
      const target = e.target;
      const action = contextMenuAction({
        isTextEdit: target !== null && isTextEditTarget(target),
        isDev: import.meta.env.DEV,
      });
      if (action === "native") return;
      e.preventDefault();
      if (action !== "edit-menu" || !(target instanceof HTMLElement)) return;
      // Keyboard invocation (menu key / Shift+F10) reports 0,0; anchor to the
      // field instead so the menu does not appear in the window corner.
      let position: MenuPosition = { x: e.clientX, y: e.clientY };
      if (e.clientX === 0 && e.clientY === 0) {
        const rect = target.getBoundingClientRect();
        position = { x: rect.left, y: rect.bottom };
      }
      setEditMenu({ target, position });
    };
    document.addEventListener("contextmenu", handler);
    return () => document.removeEventListener("contextmenu", handler);
  }, []);

  const closeEditMenu = useCallback(() => setEditMenu(null), []);

  const toggleInfoPane = () => {
    const next = !infoPaneCollapsed;
    // Optimistic: the UI flips immediately; a failed save only costs
    // persistence across restarts, so it is logged rather than reverted.
    setInfoPaneCollapsed(next);
    saveInfoPaneCollapsed(next).catch((e) => console.error(e));
  };

  useEffect(() => {
    const unlistenFolder = onMenuOpenFolderDialog(() => setFolderDialogOpen(true));
    const unlistenWbImport = onMenuOpenWbImportDialog(() => setWbImportDialogOpen(true));
    return () => {
      unlistenFolder();
      unlistenWbImport();
    };
  }, []);

  useEffect(() => {
    const unlisten = onMenuOpenTagBarDialog(() => setTagBarDialogOpen(true));
    return () => unlisten();
  }, []);

  useEffect(() => {
    const unlisten = onMenuOpenSettingsDialog(() => setSettingsDialogOpen(true));
    return () => unlisten();
  }, []);

  useEffect(() => {
    const unlisten = onMenuCheckUpdate(() => setUpdateDialogOpen(true));
    return () => unlisten();
  }, []);

  useEffect(() => {
    const unlisten = onMenuAbout(() => setAboutDialogOpen(true));
    return () => unlisten();
  }, []);

  useEffect(() => {
    const unlisten = onMenuOpenLogDialog(() => setLogDialogOpen(true));
    return () => unlisten();
  }, []);

  useEffect(() => {
    const unlistenStyle = onMenuStyleSelected((style) => {
      // Only known values are accepted; unrecognized payloads (e.g. from a
      // future menu item not yet handled here) are ignored rather than
      // blindly cast.
      if (style === "default") {
        setViewStyle(style);
      }
    });
    return () => {
      unlistenStyle();
    };
  }, []);

  return (
    <div className="app">
      {/* No <h1>: the native menu bar (rendered outside the DOM) serves as the
          title bar. */}
      <div className="header-row-primary" data-testid="header-row-primary">
        <SearchBox value={search} onChange={setSearch} />
        <SortControl
          field={sortField}
          direction={sortDirection}
          onChange={(field, direction) => {
            setSortField(field);
            setSortDirection(direction);
          }}
        />
        <button
          type="button"
          className="info-pane-toggle"
          data-testid="info-pane-toggle"
          aria-pressed={infoPaneCollapsed}
          onClick={toggleInfoPane}
        >
          {infoPaneCollapsed ? "情報を表示" : "情報を隠す"}
        </button>
      </div>
      <div className="main-area">
        <LeftPane
          folderPath={folderPath}
          onFolderPathChange={setFolderPath}
          ratings={ratings}
          onRatingsChange={setRatings}
          tagIds={tagIds}
          onTagIdsChange={setTagIds}
          refreshKey={refreshKey}
          tagBarRefreshKey={tagBarRefreshKey}
        />
        {
          // viewStyle -> component mapping, kept as a single map so a
          // future style just adds a key here.
          {
            default: (
              <ThumbnailGrid
                refreshKey={refreshKey}
                thumbnailVersions={thumbnailVersions}
                search={search}
                sortField={sortField}
                sortDirection={sortDirection}
                tagIds={tagIds}
                folderPath={folderPath}
                ratings={ratings}
                infoCollapsed={infoPaneCollapsed}
                jumpRequest={jumpRequest}
                onCountChange={setVideoCount}
                onSelectionCountChange={setSelectedCount}
              />
            ),
          }[viewStyle]
        }
      </div>
      {/* The always-visible status bar. The 未登録/重複/生成失敗 badges and
          their panels live inside StatusBar itself. */}
      <StatusBar
        refreshKey={refreshKey}
        videoCount={videoCount}
        selectedCount={selectedCount}
        onJump={(percent) => setJumpRequest((prev) => ({ percent, seq: (prev?.seq ?? 0) + 1 }))}
      />

      {/* FfmpegNotice is a top-level, always-visible notice, not a dialog. */}
      <FfmpegNotice />
      <UpdateNotice onOpenDetails={() => setUpdateDialogOpen(true)} />

      {/* FolderDialog/WbImportDialog are modals reachable only via the menu,
          not always-visible. */}
      <FolderDialog
        open={folderDialogOpen}
        onClose={() => setFolderDialogOpen(false)}
        onChanged={() => setRefreshKey((k) => k + 1)}
      />
      {/* Mounted only while open: unmounting on close is what discards the
          previous selection/result so each open starts at file selection.
          Always rendering it with an open flag would keep that state. */}
      {wbImportDialogOpen && (
        <WbImportDialog
          onClose={() => setWbImportDialogOpen(false)}
          onImportComplete={() => setRefreshKey((k) => k + 1)}
          onThumbnailsReplaced={(videoId) =>
            setThumbnailVersions((prev) => ({ ...prev, [videoId]: (prev[videoId] ?? 0) + 1 }))
          }
        />
      )}
      <TagBarEditDialog
        open={tagBarDialogOpen}
        onClose={() => setTagBarDialogOpen(false)}
        onChanged={() => setTagBarRefreshKey((k) => k + 1)}
        onTagDeleted={(tagId) => setTagIds((ids) => ids.filter((id) => id !== tagId))}
      />
      <SettingsDialog open={settingsDialogOpen} onClose={() => setSettingsDialogOpen(false)} />
      <LogViewerDialog open={logDialogOpen} onClose={() => setLogDialogOpen(false)} />
      <UpdateDialog open={updateDialogOpen} onClose={() => setUpdateDialogOpen(false)} />
      <AboutDialog open={aboutDialogOpen} onClose={() => setAboutDialogOpen(false)} />
      {editMenu && (
        <EditContextMenu
          target={editMenu.target}
          position={editMenu.position}
          onClose={closeEditMenu}
        />
      )}
    </div>
  );
}

export default App;
