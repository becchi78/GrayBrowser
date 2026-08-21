import { useEffect, useMemo, useState } from "react";
import { getTagBarPinnedTagIds, listAllTags, setTagBarPinnedTagIds } from "../api";
import { addIfAbsent, moveDown, moveUp, removeAt } from "../lib/reorderablePinnedTags";
import type { TagDto } from "../types";

interface Props {
  open: boolean;
  onClose: () => void;
  /** Bumped-parent-refetch callback, same role as FolderDialog's own
   * `onChanged` -- TagBar re-fetches its pinned list on `refreshKey`
   * change. */
  onChanged: () => void;
}

// Draft-then-save editing, same convention as FolderDialog's editingValue/
// cancelEdit: every ↑/↓/✕/+ 追加 click below only mutates local
// `draftPinnedIds` state. Nothing reaches the backend until "保存" is
// clicked; "キャンセル" (or the overlay click) discards the draft
// unpersisted -- the next `open` re-fetches a fresh draft from the
// currently-persisted list anyway, so there's nothing to explicitly reset.
export function TagBarEditDialog({ open, onClose, onChanged }: Props) {
  const [allTags, setAllTags] = useState<TagDto[]>([]);
  const [draftPinnedIds, setDraftPinnedIds] = useState<number[]>([]);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!open) {
      return;
    }
    let cancelled = false;
    Promise.all([listAllTags(), getTagBarPinnedTagIds()])
      .then(([tags, pinnedIds]) => {
        if (cancelled) return;
        setAllTags(tags);
        setDraftPinnedIds(pinnedIds);
        setError(null);
      })
      .catch((e) => {
        if (!cancelled) setError(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [open]);

  const tagsById = useMemo(() => new Map(allTags.map((tag) => [tag.id, tag])), [allTags]);

  if (!open) {
    return null;
  }

  const pinnedSet = new Set(draftPinnedIds);
  const otherTags = allTags.filter((tag) => !pinnedSet.has(tag.id));

  async function handleSave() {
    setSaving(true);
    setError(null);
    try {
      await setTagBarPinnedTagIds(draftPinnedIds);
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
        <div className="tag-bar-edit-columns">
          <div className="tag-bar-edit-column">
            <h3>バーに常設するタグ</h3>
            {draftPinnedIds.length === 0 ? (
              <p className="tag-bar-edit-empty">
                まだありません。右のリストから追加してください。
              </p>
            ) : (
              <ul className="tag-bar-edit-list" data-testid="tag-bar-edit-pinned-list">
                {draftPinnedIds.map((id, index) => {
                  const tag = tagsById.get(id);
                  if (!tag) return null;
                  return (
                    <li key={id} className="tag-bar-edit-row" data-testid="tag-bar-edit-pinned-row">
                      <span className="tag-bar-edit-row-name" title={tag.name}>
                        {tag.name}
                      </span>
                      <div className="tag-bar-edit-row-actions">
                        <button
                          type="button"
                          aria-label="上へ移動"
                          data-testid="tag-bar-edit-move-up-btn"
                          onClick={() => setDraftPinnedIds((ids) => moveUp(ids, index))}
                          disabled={saving || index === 0}
                        >
                          ↑
                        </button>
                        <button
                          type="button"
                          aria-label="下へ移動"
                          data-testid="tag-bar-edit-move-down-btn"
                          onClick={() => setDraftPinnedIds((ids) => moveDown(ids, index))}
                          disabled={saving || index === draftPinnedIds.length - 1}
                        >
                          ↓
                        </button>
                        <button
                          type="button"
                          aria-label="常設から外す"
                          data-testid="tag-bar-edit-remove-btn"
                          onClick={() => setDraftPinnedIds((ids) => removeAt(ids, index))}
                          disabled={saving}
                        >
                          ✕
                        </button>
                      </div>
                    </li>
                  );
                })}
              </ul>
            )}
          </div>
          <div className="tag-bar-edit-column">
            <h3>その他のタグ</h3>
            {otherTags.length === 0 ? (
              <p className="tag-bar-edit-empty">ありません。</p>
            ) : (
              <ul className="tag-bar-edit-list" data-testid="tag-bar-edit-other-list">
                {otherTags.map((tag) => (
                  <li key={tag.id} className="tag-bar-edit-row" data-testid="tag-bar-edit-other-row">
                    <span className="tag-bar-edit-row-name" title={tag.name}>
                      {tag.name}
                    </span>
                    <button
                      type="button"
                      data-testid="tag-bar-edit-add-btn"
                      onClick={() => setDraftPinnedIds((ids) => addIfAbsent(ids, tag.id))}
                      disabled={saving}
                    >
                      + 追加
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </div>
        </div>
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
