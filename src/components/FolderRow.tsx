interface Props {
  path: string;
  isFirst: boolean;
  isLast: boolean;
  /** 更新チェックが入っているか。 */
  checkEnabled: boolean;
  /** 並べ替えの保存中。すべての行の ↑/↓ を押せなくする。 */
  reordering: boolean;
  /** いずれかの行で更新チェックを切り替え中。すべての行のチェックを押せなくする。 */
  toggling: boolean;
  /** この行がパスの編集中か。 */
  editing: boolean;
  editingValue: string;
  editError: string | null;
  savingEdit: boolean;
  onMove: (direction: "up" | "down") => void;
  onToggleCheck: (enabled: boolean) => void;
  onStartEdit: () => void;
  onDelete: () => void;
  onEditingValueChange: (value: string) => void;
  onSaveEdit: () => void;
  onCancelEdit: () => void;
}

// フォルダ管理ダイアログの1行。通常の表示と、パスの編集中の表示を描く。
// 状態は持たず、値と操作はすべて呼び出し側から受け取る。
export function FolderRow({
  path,
  isFirst,
  isLast,
  checkEnabled,
  reordering,
  toggling,
  editing,
  editingValue,
  editError,
  savingEdit,
  onMove,
  onToggleCheck,
  onStartEdit,
  onDelete,
  onEditingValueChange,
  onSaveEdit,
  onCancelEdit,
}: Props) {
  return (
    <li className="folder-row" data-testid="folder-row">
      {editing ? (
        <div className="folder-row-edit">
          <div className="folder-row-edit-controls">
            <input
              className="folder-row-path-input"
              data-testid="folder-row-path-input"
              value={editingValue}
              onChange={(e) => onEditingValueChange(e.target.value)}
              disabled={savingEdit}
            />
            <button
              type="button"
              data-testid="folder-row-save-btn"
              onClick={onSaveEdit}
              disabled={savingEdit}
            >
              {savingEdit ? "保存中..." : "保存"}
            </button>
            <button
              type="button"
              data-testid="folder-row-cancel-btn"
              onClick={onCancelEdit}
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
            onClick={() => onMove("up")}
            disabled={isFirst || reordering}
          >
            ↑
          </button>
          <button
            type="button"
            className="folder-row-move-btn"
            data-testid="folder-row-move-down-btn"
            aria-label="下へ移動"
            onClick={() => onMove("down")}
            disabled={isLast || reordering}
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
              checked={checkEnabled}
              onChange={(e) => onToggleCheck(e.target.checked)}
              disabled={toggling}
            />
            更新チェック
          </label>
          <button
            type="button"
            className="folder-row-edit-btn"
            data-testid="folder-row-edit-btn"
            aria-label="パスを編集"
            onClick={onStartEdit}
          >
            ✎
          </button>
          <button
            type="button"
            className="folder-row-delete-btn"
            data-testid="folder-row-delete-btn"
            aria-label="フォルダの登録を解除"
            onClick={onDelete}
          >
            ✕
          </button>
        </>
      )}
    </li>
  );
}
