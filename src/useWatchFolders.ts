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
} from "./api";
import { moveDown, moveUp } from "./lib/reorderablePinnedTags";
import { buildScanSummaryMessage } from "./lib/scanSummaryMessage";
import { rescanState } from "./lib/scanTargets";

export interface DeleteTarget {
  path: string;
  count: number;
}

// フォルダ管理ダイアログの状態と操作(一覧の取得、追加、再スキャン、並べ替え、
// 更新チェックの切り替え、パスの変更、登録解除)。編集の状態は1行分だが、
// ダイアログ全体で同時に1か所しか編集できないので、ここにまとめて持つ。
export function useWatchFolders(open: boolean, onChanged: () => void) {
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

  const rescan = rescanState(folders, checkDisabled);

  // 以下の操作は useCallback で包まず、描画のたびに作り直す。そのため、
  // どの操作もその描画時点の状態を読む(たとえば handleMove は最新の
  // `folders` から次の順序を計算する)。

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

  // Folders are registered automatically when added (watcher start / diff
  // scan), so this is a manual full re-check, not a required first step. The
  // backend skips folders whose update check is off.
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

  // The backend rewrites every video path under the old folder path while
  // keeping each video's id/tags/rating/created_at intact, then replaces the
  // folder entry itself. Frontend-side validation is limited to "non-empty";
  // the backend rejects duplicates and containment, so any such rejection
  // surfaces here only as the error string it returns.
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

  function cancelDelete() {
    setDeleteTarget(null);
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

  return {
    folders,
    checkDisabled,
    rescan,
    orderCheckError,
    renameSummary,
    lastScanSummary,
    scanning,
    reordering,
    togglingPath,
    editingPath,
    editingValue,
    setEditingValue,
    editError,
    savingEdit,
    deleteTarget,
    deleting,
    handleAdd,
    handleScan,
    handleMove,
    handleToggleCheck,
    startEdit,
    cancelEdit,
    saveEdit,
    openDeleteConfirm,
    cancelDelete,
    confirmDelete,
  };
}
