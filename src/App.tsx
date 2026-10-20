import { useEffect, useState } from "react";
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
import { getInfoPaneCollapsed, setInfoPaneCollapsed as saveInfoPaneCollapsed } from "./api";
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
import type { SortDirection, SortField } from "./types";

// Only "default" exists today. New styles (e.g. "grid") get added here as
// additional literal values, plus one entry in the viewStyle -> component
// mapping below -- no other App.tsx changes.
type ViewStyle = "default";

function App() {
  const [refreshKey, setRefreshKey] = useState(0);
  const [search, setSearch] = useState("");
  const [sortField, setSortField] = useState<SortField>("created_at");
  const [sortDirection, setSortDirection] = useState<SortDirection>("asc");
  const [tagIds, setTagIds] = useState<number[]>([]);
  // `null` = "すべて" (no folder filter).
  const [folderPath, setFolderPath] = useState<string | null>(null);
  // Empty = "ALL" (no rating filter). Independent filter axis from
  // tagIds -- see RatingBar's own comment.
  const [ratings, setRatings] = useState<number[]>([]);
  // FolderDialog/WbImportDialog are reached only via the native menu
  // ("ファイル > フォルダ管理.../.wbインポート..." -- see the two
  // onMenuOpen*Dialog subscriptions below).
  const [folderDialogOpen, setFolderDialogOpen] = useState(false);
  const [wbImportDialogOpen, setWbImportDialogOpen] = useState(false);
  // TagBarEditDialog is reached only via the native menu ("タグ > タグ
  // バーの編集..." -- its own top-level submenu, not nested under
  // "ファイル", see lib.rs). tagBarRefreshKey mirrors `refreshKey`'s own
  // role, scoped to just TagBar: bumped once the dialog saves a new pinned
  // list, so TagBar re-fetches it (via its own `refreshKey` prop) without a
  // full remount.
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
  const [jumpRequest, setJumpRequest] = useState<{ percent: number; seq: number } | null>(null);

  useEffect(() => {
    getInfoPaneCollapsed()
      .then(setInfoPaneCollapsed)
      .catch((e) => console.error(e));
  }, []);

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
      {/* No more <h1> -- the native menu bar (Tauri v2, rendered outside
          the DOM) is the title-bar equivalent now. */}
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
          their panels now live inside StatusBar itself -- see that
          component's own comment. */}
      <StatusBar
        refreshKey={refreshKey}
        videoCount={videoCount}
        selectedCount={selectedCount}
        onJump={(percent) => setJumpRequest((prev) => ({ percent, seq: (prev?.seq ?? 0) + 1 }))}
      />

      {/* FfmpegNotice is not part of the dialog list, so it stays a
          top-level, always-visible notice (unlike FolderControls/
          WbImportPanel below, both of which moved into modals). */}
      <FfmpegNotice />
      <UpdateNotice onOpenDetails={() => setUpdateDialogOpen(true)} />

      {/* FolderControls/WbImportPanel's former direct App.tsx mounts are
          gone -- their logic now lives inside these two modals, reachable
          only via the menu/sidebar link above, not always-visible. */}
      <FolderDialog
        open={folderDialogOpen}
        onClose={() => setFolderDialogOpen(false)}
        onChanged={() => setRefreshKey((k) => k + 1)}
      />
      <WbImportDialog
        open={wbImportDialogOpen}
        onClose={() => setWbImportDialogOpen(false)}
        onImportComplete={() => setRefreshKey((k) => k + 1)}
      />
      <TagBarEditDialog
        open={tagBarDialogOpen}
        onClose={() => setTagBarDialogOpen(false)}
        onChanged={() => setTagBarRefreshKey((k) => k + 1)}
      />
      <SettingsDialog open={settingsDialogOpen} onClose={() => setSettingsDialogOpen(false)} />
      <LogViewerDialog open={logDialogOpen} onClose={() => setLogDialogOpen(false)} />
      <UpdateDialog open={updateDialogOpen} onClose={() => setUpdateDialogOpen(false)} />
      <AboutDialog open={aboutDialogOpen} onClose={() => setAboutDialogOpen(false)} />
    </div>
  );
}

export default App;
