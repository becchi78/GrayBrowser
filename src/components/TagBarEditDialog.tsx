import { useEffect, useState } from "react";
import { deleteTag, listTagsWithVideoCount, setTagOrder } from "../api";
import { moveDown, moveUp } from "../lib/reorderablePinnedTags";
import { buildTagDeleteMessage } from "../lib/tagDeleteMessage";
import type { TagWithCountDto } from "../types";
import { ConfirmDialog } from "./ConfirmDialog";

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

// Draft-then-save editing, same convention as FolderDialog's editingValue/
// cancelEdit: every ↑/↓ click below only mutates local `draftTags` state.
// Nothing reaches the backend until "保存" is clicked; "キャンセル" (or the
// overlay click) discards the draft unpersisted -- the next `open` re-fetches
// a fresh draft from the currently-persisted order anyway, so there's nothing
// to explicitly reset.
//
// Deleting a tag is the exception: it is applied the moment the confirmation
// is accepted, not deferred to "保存", because a delete cannot be meaningfully
// held as a draft. The deleted row is dropped from the draft so an unsaved
// reorder survives it.
export function TagBarEditDialog({ open, onClose, onChanged, onTagDeleted }: Props) {
  const [draftTags, setDraftTags] = useState<TagWithCountDto[]>([]);
  const [saving, setSaving] = useState(false);
  const [deleteTarget, setDeleteTarget] = useState<TagWithCountDto | null>(null);
  const [deleting, setDeleting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!open) {
      return;
    }
    let cancelled = false;
    listTagsWithVideoCount()
      .then((tags) => {
        if (cancelled) return;
        setDraftTags(tags);
        setError(null);
      })
      .catch((e) => {
        if (!cancelled) setError(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [open]);

  if (!open) {
    return null;
  }

  async function handleSave() {
    setSaving(true);
    setError(null);
    try {
      await setTagOrder(draftTags.map((tag) => tag.id));
      onChanged();
      onClose();
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  }

  async function handleConfirmDelete() {
    if (!deleteTarget) return;
    const target = deleteTarget;
    setDeleting(true);
    setError(null);
    try {
      await deleteTag(target.id);
      setDraftTags((tags) => tags.filter((tag) => tag.id !== target.id));
      onTagDeleted(target.id);
      onChanged();
    } catch (e) {
      setError(String(e));
    } finally {
      setDeleting(false);
      setDeleteTarget(null);
    }
  }

  // ConfirmDialog sits beside (not inside) the overlay: React bubbles events
  // through portals, so nesting it would let its overlay clicks reach this
  // dialog's own overlay-click-to-close handler.
  return (
    <>
      <div className="dialog-overlay" onClick={() => !saving && !deleting && onClose()}>
        <div
          className="dialog tag-bar-edit-dialog"
          data-testid="tag-bar-edit-dialog"
          onClick={(e) => e.stopPropagation()}
        >
          <h2>タグの編集</h2>
          {draftTags.length === 0 ? (
            <p className="tag-bar-edit-empty">タグがありません。</p>
          ) : (
            <ul className="tag-bar-edit-list" data-testid="tag-bar-edit-list">
              {draftTags.map((tag, index) => (
                <li key={tag.id} className="tag-bar-edit-row" data-testid="tag-bar-edit-row">
                  <span className="tag-bar-edit-row-name" title={tag.name}>
                    {tag.name}
                  </span>
                  <div className="tag-bar-edit-row-actions">
                    <button
                      type="button"
                      aria-label="上へ移動"
                      data-testid="tag-bar-edit-move-up-btn"
                      onClick={() => setDraftTags((tags) => moveUp(tags, index))}
                      disabled={saving || deleting || index === 0}
                    >
                      ↑
                    </button>
                    <button
                      type="button"
                      aria-label="下へ移動"
                      data-testid="tag-bar-edit-move-down-btn"
                      onClick={() => setDraftTags((tags) => moveDown(tags, index))}
                      disabled={saving || deleting || index === draftTags.length - 1}
                    >
                      ↓
                    </button>
                    <button
                      type="button"
                      data-testid="tag-bar-edit-delete-btn"
                      onClick={() => setDeleteTarget(tag)}
                      disabled={saving || deleting}
                    >
                      削除
                    </button>
                  </div>
                </li>
              ))}
            </ul>
          )}
          {error && (
            <p className="tag-bar-edit-error" data-testid="tag-bar-edit-error">
              {error}
            </p>
          )}
          <div className="dialog-footer">
            <button
              type="button"
              data-testid="tag-bar-edit-cancel-btn"
              onClick={onClose}
              disabled={saving || deleting}
            >
              キャンセル
            </button>
            <button
              type="button"
              data-testid="tag-bar-edit-save-btn"
              onClick={handleSave}
              disabled={saving || deleting}
            >
              {saving ? "保存中..." : "保存"}
            </button>
          </div>
        </div>
      </div>
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
          <>
            <p className="tag-delete-target" data-testid="tag-delete-target">
              {deleteTarget.name}
            </p>
            <p>{buildTagDeleteMessage(deleteTarget.video_count)}</p>
          </>
        )}
      </ConfirmDialog>
    </>
  );
}
