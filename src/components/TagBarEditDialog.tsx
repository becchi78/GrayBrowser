import { useEffect, useState } from "react";
import { listAllTags, setTagOrder } from "../api";
import { moveDown, moveUp } from "../lib/reorderablePinnedTags";
import type { TagDto } from "../types";

interface Props {
  open: boolean;
  onClose: () => void;
  /** Bumped-parent-refetch callback, same role as FolderDialog's own
   * `onChanged` -- TagBar re-fetches its tag order on `refreshKey`
   * change. */
  onChanged: () => void;
}

// Draft-then-save editing, same convention as FolderDialog's editingValue/
// cancelEdit: every ↑/↓ click below only mutates local `draftTags` state.
// Nothing reaches the backend until "保存" is clicked; "キャンセル" (or the
// overlay click) discards the draft unpersisted -- the next `open` re-fetches
// a fresh draft from the currently-persisted order anyway, so there's nothing
// to explicitly reset.
export function TagBarEditDialog({ open, onClose, onChanged }: Props) {
  const [draftTags, setDraftTags] = useState<TagDto[]>([]);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!open) {
      return;
    }
    let cancelled = false;
    listAllTags()
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

  return (
    <div className="dialog-overlay" onClick={() => !saving && onClose()}>
      <div
        className="dialog tag-bar-edit-dialog"
        data-testid="tag-bar-edit-dialog"
        onClick={(e) => e.stopPropagation()}
      >
        <h2>タグバーの編集</h2>
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
                    disabled={saving || index === 0}
                  >
                    ↑
                  </button>
                  <button
                    type="button"
                    aria-label="下へ移動"
                    data-testid="tag-bar-edit-move-down-btn"
                    onClick={() => setDraftTags((tags) => moveDown(tags, index))}
                    disabled={saving || index === draftTags.length - 1}
                  >
                    ↓
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
            disabled={saving}
          >
            キャンセル
          </button>
          <button
            type="button"
            data-testid="tag-bar-edit-save-btn"
            onClick={handleSave}
            disabled={saving}
          >
            {saving ? "保存中..." : "保存"}
          </button>
        </div>
      </div>
    </div>
  );
}
