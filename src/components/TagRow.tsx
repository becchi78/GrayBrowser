import type { TagWithCountDto } from "../types";
import { TrashIcon } from "./SidebarIcons";

interface Props {
  tag: TagWithCountDto;
  /** グループの中の行か。インデントだけが変わる。 */
  grouped: boolean;
  disableUp: boolean;
  disableDown: boolean;
  /** 保存中・削除中。すべてのボタンを押せなくする。 */
  busy: boolean;
  /** この行が名前の編集中か。 */
  editing: boolean;
  editingValue: string;
  editError: string | null;
  onMove: (direction: "up" | "down") => void;
  onStartEdit: () => void;
  onDelete: () => void;
  onEditingValueChange: (value: string) => void;
  onSaveEdit: () => void;
  onCancelEdit: () => void;
}

// タグの編集画面の、タグ1行。状態は持たず、値と操作は呼び出し側から受け取る。
export function TagRow({
  tag,
  grouped,
  disableUp,
  disableDown,
  busy,
  editing,
  editingValue,
  editError,
  onMove,
  onStartEdit,
  onDelete,
  onEditingValueChange,
  onSaveEdit,
  onCancelEdit,
}: Props) {
  return (
    <li
      className={grouped ? "tag-bar-edit-row tag-bar-edit-row--grouped" : "tag-bar-edit-row"}
      data-testid="tag-bar-edit-row"
    >
      {editing ? (
        <div className="tag-bar-edit-group-edit">
          <div className="tag-bar-edit-input-controls">
            <input
              className="tag-bar-edit-name-input"
              data-testid="tag-bar-edit-tag-name-input"
              aria-label="タグ名"
              value={editingValue}
              onChange={(e) => onEditingValueChange(e.target.value)}
              disabled={busy}
            />
            <button
              type="button"
              data-testid="tag-bar-edit-tag-save-btn"
              onClick={onSaveEdit}
              disabled={busy}
            >
              {busy ? "保存中..." : "保存"}
            </button>
            <button
              type="button"
              data-testid="tag-bar-edit-tag-cancel-btn"
              onClick={onCancelEdit}
              disabled={busy}
            >
              キャンセル
            </button>
          </div>
          {editError && (
            <p className="tag-bar-edit-error" data-testid="tag-bar-edit-tag-edit-error">
              {editError}
            </p>
          )}
        </div>
      ) : (
        <>
          <button
            type="button"
            className="tag-bar-edit-move-btn"
            aria-label="上へ移動"
            data-testid="tag-bar-edit-move-up-btn"
            onClick={() => onMove("up")}
            disabled={busy || disableUp}
          >
            ↑
          </button>
          <button
            type="button"
            className="tag-bar-edit-move-btn"
            aria-label="下へ移動"
            data-testid="tag-bar-edit-move-down-btn"
            onClick={() => onMove("down")}
            disabled={busy || disableDown}
          >
            ↓
          </button>
          <span className="tag-bar-edit-row-name" title={tag.name}>
            {tag.name}
          </span>
          <button
            type="button"
            className="tag-bar-edit-move-btn"
            aria-label="タグ名を編集"
            data-testid="tag-bar-edit-tag-edit-btn"
            onClick={onStartEdit}
            disabled={busy}
          >
            ✎
          </button>
          <button
            type="button"
            className="tag-bar-edit-delete-btn"
            aria-label="タグを削除"
            data-testid="tag-bar-edit-delete-btn"
            onClick={onDelete}
            disabled={busy}
          >
            <TrashIcon className="delete-btn-icon" />
          </button>
        </>
      )}
    </li>
  );
}
