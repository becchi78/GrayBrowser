import { useWatchFolders } from "../useWatchFolders";
import { ConfirmDialog } from "./ConfirmDialog";
import { FolderRow } from "./FolderRow";
import { ModalShell } from "./ModalShell";

interface Props {
  open: boolean;
  onClose: () => void;
  /** Called whenever a folder is added, removed, or (re-)scanned, so the
   * parent can bump its refresh key and the sidebar/grid/status bar pick it
   * up without a full remount. */
  onChanged: () => void;
}

export function FolderDialog({ open, onClose, onChanged }: Props) {
  const {
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
  } = useWatchFolders(open, onChanged);

  if (!open) {
    return null;
  }

  return (
    <>
      <ModalShell
        label="フォルダ管理"
        className="folder-dialog"
        testId="folder-dialog"
        onDismiss={onClose}
        // 登録解除の確認が開いている間は、Escape で確認だけを閉じる。
        dismissible={deleteTarget === null}
      >
        <h2>フォルダ管理</h2>
        {folders.length === 0 ? (
          <p className="folder-dialog-empty">
            登録済みのフォルダはありません。
          </p>
        ) : (
          <ul className="folder-dialog-list">
            {folders.map((path, index) => (
              <FolderRow
                key={path}
                path={path}
                isFirst={index === 0}
                isLast={index === folders.length - 1}
                checkEnabled={!checkDisabled.includes(path)}
                reordering={reordering}
                toggling={togglingPath !== null}
                editing={editingPath === path}
                editingValue={editingValue}
                editError={editError}
                savingEdit={savingEdit}
                onMove={(direction) => handleMove(index, direction)}
                onToggleCheck={(enabled) => handleToggleCheck(path, enabled)}
                onStartEdit={() => startEdit(path)}
                onDelete={() => openDeleteConfirm(path)}
                onEditingValueChange={setEditingValue}
                onSaveEdit={saveEdit}
                onCancelEdit={cancelEdit}
              />
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
        <div className="folder-dialog-scan-row">
          <button
            type="button"
            data-testid="folder-dialog-scan-btn"
            onClick={handleScan}
            disabled={scanning || !rescan.enabled}
          >
            {scanning ? "スキャン中..." : "再スキャン"}
          </button>
          {lastScanSummary && (
            <span className="scan-summary">{lastScanSummary}</span>
          )}
        </div>
        <p
          className="folder-dialog-scan-note"
          data-testid="folder-dialog-scan-note"
        >
          {rescan.enabled
            ? "「更新チェック」が入っているフォルダを、あらためて全件確認します。チェックを外したフォルダは対象外です。"
            : rescan.reason === "all-check-off"
              ? "すべてのフォルダで「更新チェック」が外れているため、再スキャンできません。"
              : "フォルダが登録されていないため、再スキャンできません。"}
        </p>
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
      </ModalShell>

      {/* A sibling of the ModalShell, not a child; see ModalShell for why. */}
      <ConfirmDialog
        open={deleteTarget !== null}
        title="フォルダの登録を解除"
        confirmLabel="登録を解除する"
        danger
        busy={deleting}
        onConfirm={confirmDelete}
        onCancel={cancelDelete}
      >
        {deleteTarget && (
          <>
            <p>
              このフォルダの登録を解除します。配下の{" "}
              <strong>{deleteTarget.count}</strong>{" "}
              件の動画について、GrayBrowser上のタグ・レーティング・登録日の情報が失われます（動画ファイル自体は削除されません）。
            </p>
            <p className="folder-delete-confirm-path">{deleteTarget.path}</p>
          </>
        )}
      </ConfirmDialog>
    </>
  );
}
