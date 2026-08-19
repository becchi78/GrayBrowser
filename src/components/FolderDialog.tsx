import { useEffect, useState } from "react";
import {
  countVideosUnderFolder,
  listWatchFolders,
  pickWatchFolders,
  removeWatchFolder,
  renameWatchFolder,
  startScan,
} from "../api";

interface Props {
  open: boolean;
  onClose: () => void;
  /** Bumped by the parent (mirrors the old FolderControls' onScanComplete/
   * WbImportPanel's onImportComplete callbacks) whenever a folder is added,
   * removed, or (re-)scanned, so the sidebar/grid/status bar pick it up
   * without a full remount. */
  onChanged: () => void;
}

interface DeleteTarget {
  path: string;
  count: number;
}

// Displays the watch mode (リアルタイム or NASポーリング) for each folder.
//
// The *real* classification lives entirely in the Windows adapter layer
// (`gb_core::ports::drive_type::DriveTypeDetector`, resolved via the real
// `GetDriveTypeW` call in `src-tauri::adapters::drive_type`) and is only
// ever consulted by `RealtimeWatchManager::reconfigure` when folders are
// actually (re)configured for watching -- there is no Tauri command that
// exposes it for display purposes (a read-only informational label does
// not need a live Win32 API round trip). This is therefore a
// frontend-only approximation: a UNC path (`\\server\share\...`) is
// always NAS-polled by `RealtimeWatchManager::reconfigure`, matching the
// same UNC-vs-drive-letter shape `gb_core::paths::extract_drive_root`
// already recognizes structurally (see that function's own doc comment).
// Known limitation: a *mapped* network drive letter (e.g. `Z:\` backed by a
// network share) cannot be told apart from a genuinely local drive without
// the real `GetDriveTypeW` call, so this heuristic would mislabel it
// "リアルタイム".
function isNetworkPath(path: string): boolean {
  return path.startsWith("\\\\") || path.startsWith("//");
}

export function FolderDialog({ open, onClose, onChanged }: Props) {
  const [folders, setFolders] = useState<string[]>([]);
  const [editingPath, setEditingPath] = useState<string | null>(null);
  const [editingValue, setEditingValue] = useState("");
  const [editError, setEditError] = useState<string | null>(null);
  const [savingEdit, setSavingEdit] = useState(false);
  const [renameSummary, setRenameSummary] = useState<string | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<DeleteTarget | null>(null);
  const [deleting, setDeleting] = useState(false);
  const [scanning, setScanning] = useState(false);
  const [lastScanSummary, setLastScanSummary] = useState<string | null>(null);

  useEffect(() => {
    if (!open) {
      return;
    }
    let cancelled = false;
    listWatchFolders()
      .then((rows) => {
        if (!cancelled) setFolders(rows);
      })
      .catch(console.error);
    return () => {
      cancelled = true;
    };
  }, [open]);

  if (!open) {
    return null;
  }

  async function handleAdd() {
    try {
      const merged = await pickWatchFolders();
      setFolders(merged);
      onChanged();
    } catch (e) {
      console.error(e);
    }
  }

  // FolderDialog's footer doesn't list a scan-trigger button in the design
  // spec (only "+ フォルダを追加" / "閉じる"). But `pick_watch_folders`
  // (handleAdd above) only persists the folder and reconfigures the
  // realtime watcher/NAS poller -- `RealtimeWatchManager::reconfigure`
  // starts watching for *future* filesystem events, it does not enumerate a
  // newly added folder's *existing* files. `start_scan` is the only thing
  // that does (`gb_core`'s full walk + register pipeline), and there is no
  // other entry point anywhere in the app that calls it. Dropping this
  // button with no replacement would silently remove the app's only way to
  // populate a folder's pre-existing videos.
  async function handleScan() {
    setScanning(true);
    setLastScanSummary(null);
    try {
      const summary = await startScan();
      setLastScanSummary(
        `検出: ${summary.scanned} / 登録: ${summary.registered} / 更新: ${summary.reconciled} / ` +
          `変化なし: ${summary.unchanged} / スキップ: ${summary.skipped} / ` +
          `オフライン化: ${summary.went_offline} / 復帰: ${summary.reactivated}`,
      );
      onChanged();
    } catch (e) {
      console.error(e);
    } finally {
      setScanning(false);
    }
  }

  function startEdit(path: string) {
    setEditingPath(path);
    setEditingValue(path);
    setEditError(null);
    setRenameSummary(null);
  }

  function cancelEdit() {
    setEditingPath(null);
    setEditingValue("");
    setEditError(null);
  }

  // Persists the rewritten `path` via `rename_watch_folder`, which rewrites
  // every `videos.file_path` under the old folder path while keeping each
  // row's id/tags/rating/created_at intact
  // (`queries::rename_watch_folder_videos`), then replaces the folder
  // entry itself. Frontend-side validation is limited to "non-empty"
  // -- duplicate/containment rejection is `validate_rename_target`'s job on
  // the backend, so any such rejection surfaces here only as the error
  // string it returns.
  async function saveEdit() {
    if (!editingPath) {
      return;
    }
    const trimmed = editingValue.trim();
    if (!trimmed) {
      setEditError("パスを入力してください");
      return;
    }
    setEditError(null);
    setSavingEdit(true);
    try {
      const result = await renameWatchFolder(editingPath, trimmed);
      setFolders(result.folders);
      setRenameSummary(
        result.collision_skipped_count > 0
          ? `${result.renamed_count}件のパスを更新しました（${result.collision_skipped_count}件は既存の登録と衝突したためスキップされました）`
          : `${result.renamed_count}件のパスを更新しました`,
      );
      cancelEdit();
      onChanged();
    } catch (e) {
      // Surfaced in the UI, not just console.error -- e.g. the backend's
      // duplicate/containment rejection (validate_rename_target) or the
      // folder no longer being registered.
      setEditError(String(e));
    } finally {
      setSavingEdit(false);
    }
  }

  async function openDeleteConfirm(path: string) {
    try {
      const count = await countVideosUnderFolder(path);
      setDeleteTarget({ path, count });
    } catch (e) {
      console.error(e);
    }
  }

  async function confirmDelete() {
    if (!deleteTarget) {
      return;
    }
    setDeleting(true);
    try {
      const remaining = await removeWatchFolder(deleteTarget.path);
      setFolders(remaining);
      setDeleteTarget(null);
      onChanged();
    } catch (e) {
      console.error(e);
    } finally {
      setDeleting(false);
    }
  }

  return (
    <>
      <div className="dialog-overlay" onClick={onClose}>
        <div
          className="dialog folder-dialog"
          data-testid="folder-dialog"
          onClick={(e) => e.stopPropagation()}
        >
          <h2>フォルダ管理</h2>
          {folders.length === 0 ? (
            <p className="folder-dialog-empty">登録済みのフォルダはありません。</p>
          ) : (
            <ul className="folder-dialog-list">
              {folders.map((path) => (
                <li key={path} className="folder-row" data-testid="folder-row">
                  {editingPath === path ? (
                    <div className="folder-row-edit">
                      <div className="folder-row-edit-controls">
                        <input
                          className="folder-row-path-input"
                          data-testid="folder-row-path-input"
                          value={editingValue}
                          onChange={(e) => setEditingValue(e.target.value)}
                          disabled={savingEdit}
                        />
                        <button
                          type="button"
                          data-testid="folder-row-save-btn"
                          onClick={saveEdit}
                          disabled={savingEdit}
                        >
                          {savingEdit ? "保存中..." : "保存"}
                        </button>
                        <button
                          type="button"
                          data-testid="folder-row-cancel-btn"
                          onClick={cancelEdit}
                          disabled={savingEdit}
                        >
                          キャンセル
                        </button>
                      </div>
                      {editError && (
                        <p className="folder-row-edit-error" data-testid="folder-row-edit-error">
                          {editError}
                        </p>
                      )}
                    </div>
                  ) : (
                    <>
                      <span className="folder-row-path" data-testid="folder-row-path" title={path}>
                        {path}
                      </span>
                      <span className="folder-row-mode">
                        {isNetworkPath(path) ? "NASポーリング" : "リアルタイム"}
                      </span>
                      <button
                        type="button"
                        className="folder-row-edit-btn"
                        data-testid="folder-row-edit-btn"
                        aria-label="パスを編集"
                        onClick={() => startEdit(path)}
                      >
                        ✎
                      </button>
                      <button
                        type="button"
                        className="folder-row-delete-btn"
                        data-testid="folder-row-delete-btn"
                        aria-label="フォルダの登録を解除"
                        onClick={() => openDeleteConfirm(path)}
                      >
                        ✕
                      </button>
                    </>
                  )}
                </li>
              ))}
            </ul>
          )}
          <p className="folder-dialog-note">
            ✎ パス編集は既存レコードを維持／✕ 登録解除は確認ダイアログを必須とし、失われるカタログ情報の件数を表示する
          </p>
          {renameSummary && (
            <p className="folder-dialog-rename-summary" data-testid="folder-dialog-rename-summary">
              {renameSummary}
            </p>
          )}
          {/* See handleScan's own comment: not in the design spec's listed
              footer elements, kept as a conservative safety addition --
              start_scan has no other entry point anywhere in the app. */}
          <div className="folder-dialog-scan-row">
            <button
              type="button"
              data-testid="folder-dialog-scan-btn"
              onClick={handleScan}
              disabled={scanning || folders.length === 0}
            >
              {scanning ? "スキャン中..." : "スキャン開始"}
            </button>
            {lastScanSummary && <span className="scan-summary">{lastScanSummary}</span>}
          </div>
          <div className="dialog-footer">
            <button type="button" data-testid="folder-dialog-add-btn" onClick={handleAdd}>
              + フォルダを追加
            </button>
            <button type="button" data-testid="folder-dialog-close-btn" onClick={onClose}>
              閉じる
            </button>
          </div>
        </div>
      </div>

      {deleteTarget && (
        <div className="dialog-overlay" onClick={() => !deleting && setDeleteTarget(null)}>
          <div
            className="dialog folder-delete-confirm-dialog"
            data-testid="folder-delete-confirm-dialog"
            onClick={(e) => e.stopPropagation()}
          >
            <p>
              このフォルダの登録を解除します。配下の <strong>{deleteTarget.count}</strong>{" "}
              件の動画について、GrayBrowser上のタグ・評価・登録日の情報が失われます（動画ファイル自体は削除されません）。
            </p>
            <p className="folder-delete-confirm-path">{deleteTarget.path}</p>
            <div className="dialog-footer">
              <button
                type="button"
                data-testid="folder-delete-cancel-btn"
                onClick={() => setDeleteTarget(null)}
                disabled={deleting}
              >
                キャンセル
              </button>
              <button
                type="button"
                data-testid="folder-delete-confirm-btn"
                onClick={confirmDelete}
                disabled={deleting}
              >
                {deleting ? "解除中..." : "登録を解除する"}
              </button>
            </div>
          </div>
        </div>
      )}
    </>
  );
}
