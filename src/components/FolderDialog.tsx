import { useEffect, useState } from "react";
import {
  countVideosUnderFolder,
  listWatchCheckDisabledFolders,
  listWatchFolders,
  pickWatchFolders,
  removeWatchFolder,
  renameWatchFolder,
  reorderWatchFolders,
  setWatchFolderCheckEnabled,
  startScan,
} from "../api";
import { moveDown, moveUp } from "../lib/reorderablePinnedTags";
import { buildScanSummaryMessage } from "../lib/scanSummaryMessage";

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
  const [checkDisabled, setCheckDisabled] = useState<string[]>([]);
  const [reordering, setReordering] = useState(false);
  const [togglingPath, setTogglingPath] = useState<string | null>(null);
  const [orderCheckError, setOrderCheckError] = useState<string | null>(null);

  useEffect(() => {
    if (!open) {
      return;
    }
    let cancelled = false;
    // Independent so one failing call does not keep the other list stale.
    listWatchFolders()
      .then((rows) => {
        if (!cancelled) {
          // A stale error from a previous open must not outlive the reopen.
          setOrderCheckError(null);
          setFolders(rows);
        }
      })
      .catch(console.error);
    listWatchCheckDisabledFolders()
      .then((disabled) => {
        if (!cancelled) {
          setCheckDisabled(disabled);
        }
      })
      .catch(console.error);
    return () => {
      cancelled = true;
    };
  }, [open]);

  if (!open) {
    return null;
  }

  // The backend rewrites the check-disabled list on rename/remove/add, so the
  // dialog refetches it rather than mirroring the change. A failed refetch
  // must not block the other updates.
  async function refreshCheckDisabled() {
    try {
      setCheckDisabled(await listWatchCheckDisabledFolders());
    } catch (e) {
      console.error(e);
    }
  }

  async function handleAdd() {
    try {
      const merged = await pickWatchFolders();
      setFolders(merged);
      onChanged();
      await refreshCheckDisabled();
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
      setLastScanSummary(buildScanSummaryMessage(summary));
      onChanged();
    } catch (e) {
      console.error(e);
    } finally {
      setScanning(false);
    }
  }

  // The displayed order is only replaced by the order the backend returns, so
  // a rejected save (e.g. the list changed underneath this dialog) leaves it
  // untouched. `reordering` blocks overlapping saves, which would otherwise be
  // computed from a stale list.
  async function handleMove(index: number, direction: "up" | "down") {
    const next =
      direction === "up" ? moveUp(folders, index) : moveDown(folders, index);
    if (next === folders) {
      return;
    }
    setOrderCheckError(null);
    setReordering(true);
    try {
      const saved = await reorderWatchFolders(next);
      setFolders(saved);
      onChanged();
    } catch (e) {
      setOrderCheckError(String(e));
    } finally {
      setReordering(false);
    }
  }

  async function handleToggleCheck(path: string, enabled: boolean) {
    setOrderCheckError(null);
    setTogglingPath(path);
    try {
      const disabled = await setWatchFolderCheckEnabled(path, enabled);
      setCheckDisabled(disabled);
    } catch (e) {
      setOrderCheckError(String(e));
    } finally {
      setTogglingPath(null);
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
      await refreshCheckDisabled();
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
      await refreshCheckDisabled();
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
            <p className="folder-dialog-empty">
              登録済みのフォルダはありません。
            </p>
          ) : (
            <ul className="folder-dialog-list">
              {folders.map((path, index) => (
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
                        <p
                          className="folder-row-edit-error"
                          data-testid="folder-row-edit-error"
                        >
                          {editError}
                        </p>
                      )}
                    </div>
                  ) : (
                    <>
                      <button
                        type="button"
                        className="folder-row-move-btn"
                        data-testid="folder-row-move-up-btn"
                        aria-label="上へ移動"
                        onClick={() => handleMove(index, "up")}
                        disabled={index === 0 || reordering}
                      >
                        ↑
                      </button>
                      <button
                        type="button"
                        className="folder-row-move-btn"
                        data-testid="folder-row-move-down-btn"
                        aria-label="下へ移動"
                        onClick={() => handleMove(index, "down")}
                        disabled={index === folders.length - 1 || reordering}
                      >
                        ↓
                      </button>
                      <span
                        className="folder-row-path"
                        data-testid="folder-row-path"
                        title={path}
                      >
                        {path}
                      </span>
                      <label className="folder-row-check">
                        <input
                          type="checkbox"
                          data-testid="folder-row-check-toggle"
                          checked={!checkDisabled.includes(path)}
                          onChange={(e) =>
                            handleToggleCheck(path, e.target.checked)
                          }
                          disabled={togglingPath !== null}
                        />
                        更新チェック
                      </label>
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
          {orderCheckError && (
            <p
              className="folder-dialog-order-check-error"
              data-testid="folder-dialog-order-check-error"
            >
              {orderCheckError}
            </p>
          )}
          {renameSummary && (
            <p
              className="folder-dialog-rename-summary"
              data-testid="folder-dialog-rename-summary"
            >
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
            {lastScanSummary && (
              <span className="scan-summary">{lastScanSummary}</span>
            )}
          </div>
          <div className="dialog-footer">
            <button
              type="button"
              data-testid="folder-dialog-add-btn"
              onClick={handleAdd}
              disabled={reordering || togglingPath !== null}
            >
              + フォルダを追加
            </button>
            <button
              type="button"
              data-testid="folder-dialog-close-btn"
              onClick={onClose}
            >
              閉じる
            </button>
          </div>
        </div>
      </div>

      {deleteTarget && (
        <div
          className="dialog-overlay"
          onClick={() => !deleting && setDeleteTarget(null)}
        >
          <div
            className="dialog folder-delete-confirm-dialog"
            data-testid="folder-delete-confirm-dialog"
            onClick={(e) => e.stopPropagation()}
          >
            <p>
              このフォルダの登録を解除します。配下の{" "}
              <strong>{deleteTarget.count}</strong>{" "}
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
