import type { TagWithCountDto } from "../types";

interface Props {
  tag: TagWithCountDto;
  /** グループの中の行か。インデントだけが変わる。 */
  grouped: boolean;
  disableUp: boolean;
  disableDown: boolean;
  /** 保存中・削除中。すべてのボタンを押せなくする。 */
  busy: boolean;
  onMove: (direction: "up" | "down") => void;
  onDelete: () => void;
}

// タグの編集画面の、タグ1行。状態は持たない。
export function TagRow({ tag, grouped, disableUp, disableDown, busy, onMove, onDelete }: Props) {
  return (
    <li
      className={grouped ? "tag-bar-edit-row tag-bar-edit-row--grouped" : "tag-bar-edit-row"}
      data-testid="tag-bar-edit-row"
    >
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
        className="tag-bar-edit-delete-btn"
        aria-label="タグを削除"
        data-testid="tag-bar-edit-delete-btn"
        onClick={onDelete}
        disabled={busy}
      >
        ✕
      </button>
    </li>
  );
}
