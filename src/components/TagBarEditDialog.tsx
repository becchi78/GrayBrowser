import { useEffect, useState } from "react";
import { deleteTag, listTagsWithVideoCount, setTagOrder } from "../api";
import { moveDown, moveUp } from "../lib/reorder";
import { buildTagDeleteMessage } from "../lib/tagDeleteMessage";
import type { TagWithCountDto } from "../types";
import { ConfirmDialog } from "./ConfirmDialog";
import { ModalShell } from "./ModalShell";
import { errorText } from "../lib/errorText";

interface Props {
  open: boolean;
  onClose: () => void;
  /** Bumped-parent-refetch callback, same role as FolderDialog's own
   * `onChanged` -- TagBar re-fetches its tag order on `refreshKey`
   * change. */
  onChanged: () => void;
  /** Lets the parent drop the id from any active tag filter; a deleted tag
   * left in the filter would keep the video list filtered by an id that no
   * longer exists. */
  onTagDeleted: (tagId: number) => void;
}

// Every ↑/↓ click is saved immediately, same convention as FolderDialog's
// handleMove. The list is re-fetched after a save rather than mirrored locally:
// the backend resets any tag missing from the submitted order, so re-reading
// keeps the display equal to the persisted state. On failure the displayed list
// is left untouched, which is exactly the persisted order.
export function TagBarEditDialog({ open, onClose, onChanged, onTagDeleted }: Props) {
  const [tags, setTags] = useState<TagWithCountDto[]>([]);
  const [reordering, setReordering] = useState(false);
  const [deleteTarget, setDeleteTarget] = useState<TagWithCountDto | null>(null);
  const [deleting, setDeleting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!open) {
      return;
    }
    let cancelled = false;
    listTagsWithVideoCount()
      .then((fetched) => {
        if (cancelled) return;
        setTags(fetched);
        setError(null);
      })
      .catch((e) => {
        if (!cancelled) setError(errorText(e));
      });
    return () => {
      cancelled = true;
    };
  }, [open]);

  if (!open) {
    return null;
  }

  async function handleMove(index: number, direction: "up" | "down") {
    const next = direction === "up" ? moveUp(tags, index) : moveDown(tags, index);
    if (next === tags) {
      return;
    }
    setError(null);
    setReordering(true);
    try {
      await setTagOrder(next.map((tag) => tag.id));
      // 並びは保存済み。後続の取り直しが失敗しても、左ペインには反映させる。
      onChanged();
      setTags(await listTagsWithVideoCount());
    } catch (e) {
      setError(errorText(e));
    } finally {
      setReordering(false);
    }
  }

  async function handleConfirmDelete() {
    if (!deleteTarget) return;
    const target = deleteTarget;
    setDeleting(true);
    setError(null);
    try {
      await deleteTag(target.id);
      setTags((current) => current.filter((tag) => tag.id !== target.id));
      onTagDeleted(target.id);
      onChanged();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setDeleting(false);
      setDeleteTarget(null);
    }
  }

  // ConfirmDialog sits beside (not inside) the ModalShell; see ModalShell for why.
  return (
    <>
      <ModalShell
        label="タグの編集"
        className="tag-bar-edit-dialog"
        testId="tag-bar-edit-dialog"
        onDismiss={onClose}
        // 削除の確認が開いている間は、Escape で確認だけを閉じる。
        dismissible={!reordering && !deleting && deleteTarget === null}
      >
        <h2>タグの編集</h2>
        {tags.length === 0 ? (
          <p className="tag-bar-edit-empty">タグがありません。</p>
        ) : (
          <ul className="tag-bar-edit-list" data-testid="tag-bar-edit-list">
            {tags.map((tag, index) => (
              <li key={tag.id} className="tag-bar-edit-row" data-testid="tag-bar-edit-row">
                <button
                  type="button"
                  className="tag-bar-edit-move-btn"
                  aria-label="上へ移動"
                  data-testid="tag-bar-edit-move-up-btn"
                  onClick={() => handleMove(index, "up")}
                  disabled={reordering || deleting || index === 0}
                >
                  ↑
                </button>
                <button
                  type="button"
                  className="tag-bar-edit-move-btn"
                  aria-label="下へ移動"
                  data-testid="tag-bar-edit-move-down-btn"
                  onClick={() => handleMove(index, "down")}
                  disabled={reordering || deleting || index === tags.length - 1}
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
                  onClick={() => setDeleteTarget(tag)}
                  disabled={reordering || deleting}
                >
                  ✕
                </button>
              </li>
            ))}
          </ul>
        )}
        {error && (
          <p className="tag-bar-edit-error" data-testid="tag-bar-edit-error">
            {error}
          </p>
        )}
        <div className="dialog-footer tag-bar-edit-footer">
          <button
            type="button"
            data-testid="tag-bar-edit-close-btn"
            onClick={onClose}
            disabled={reordering || deleting}
          >
            閉じる
          </button>
        </div>
      </ModalShell>
      <ConfirmDialog
        open={deleteTarget !== null}
        title="タグを削除"
        confirmLabel="削除する"
        danger
        busy={deleting}
        onConfirm={handleConfirmDelete}
        onCancel={() => setDeleteTarget(null)}
      >
        {deleteTarget && (
          <p>
            タグ『<strong className="tag-delete-target" data-testid="tag-delete-target">{deleteTarget.name}</strong>』を削除します。
            {buildTagDeleteMessage(deleteTarget.video_count)}
          </p>
        )}
      </ConfirmDialog>
    </>
  );
}
