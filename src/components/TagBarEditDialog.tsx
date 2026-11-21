import { Fragment, useEffect, useState } from "react";
import {
  createTagGroup,
  deleteTag,
  deleteTagGroup,
  getTagLayout,
  moveTagLayoutItem,
  renameTag,
  renameTagGroup,
} from "../api";
import { buildTagDeleteMessage } from "../lib/tagDeleteMessage";
import type { TagLayoutItemDto, TagWithCountDto } from "../types";
import { ConfirmDialog } from "./ConfirmDialog";
import { ModalShell } from "./ModalShell";
import { TagGroupRow } from "./TagGroupRow";
import { TagRow } from "./TagRow";
import { errorText } from "../lib/errorText";

interface Props {
  open: boolean;
  onClose: () => void;
  /** Bumped-parent-refetch callback, same role as FolderDialog's own
   * `onChanged` -- TagBar re-fetches its layout on `refreshKey` change. */
  onChanged: () => void;
  /** Lets the parent drop the id from any active tag filter; a deleted tag
   * left in the filter would keep the video list filtered by an id that no
   * longer exists. */
  onTagDeleted: (tagId: number) => void;
}

// Every change is saved immediately, same convention as FolderDialog's
// handleMove. Where to move an item (including in and out of a group) is decided
// by the backend, which returns the whole new layout; replacing the display with
// that result keeps it equal to the persisted state. On failure the displayed
// layout is left untouched, which is exactly the persisted one.
export function TagBarEditDialog({ open, ...rest }: Props) {
  // 親は常時マウントして open だけ切り替える。本体を開くたびに作り直すことで、
  // 前回の入力状態や一覧(取得に失敗したときに残ってしまう)を確実に捨てる。
  return open ? <TagBarEditDialogBody {...rest} /> : null;
}

function TagBarEditDialogBody({ onClose, onChanged, onTagDeleted }: Omit<Props, "open">) {
  const [layout, setLayout] = useState<TagLayoutItemDto[]>([]);
  const [saving, setSaving] = useState(false);
  const [deleteTarget, setDeleteTarget] = useState<TagWithCountDto | null>(null);
  const [groupDeleteTarget, setGroupDeleteTarget] = useState<number | null>(null);
  const [deleting, setDeleting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const [creating, setCreating] = useState(false);
  const [createValue, setCreateValue] = useState("");
  const [createError, setCreateError] = useState<string | null>(null);

  // 名前の編集中の行は、グループとタグを通して同時に1つだけ。作成の入力行とも排他。
  const [editing, setEditing] = useState<{ kind: "tag" | "group"; id: number } | null>(null);
  const [editingValue, setEditingValue] = useState("");
  const [editError, setEditError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    getTagLayout()
      .then((fetched) => {
        if (cancelled) return;
        setLayout(fetched);
        setError(null);
      })
      .catch((e) => {
        if (!cancelled) setError(errorText(e));
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const busy = saving || deleting;

  async function handleMove(kind: "tag" | "group", id: number, direction: "up" | "down") {
    setError(null);
    setSaving(true);
    try {
      setLayout(await moveTagLayoutItem(kind, id, direction));
      onChanged();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setSaving(false);
    }
  }

  function startCreate() {
    setEditing(null);
    setEditError(null);
    setCreateValue("");
    setCreateError(null);
    setCreating(true);
  }

  async function handleCreate() {
    setCreateError(null);
    setSaving(true);
    try {
      setLayout(await createTagGroup(createValue));
      setCreating(false);
      onChanged();
    } catch (e) {
      setCreateError(errorText(e));
    } finally {
      setSaving(false);
    }
  }

  function startEdit(kind: "tag" | "group", id: number, name: string) {
    setCreating(false);
    setCreateError(null);
    setEditing({ kind, id });
    setEditingValue(name);
    setEditError(null);
  }

  async function handleSaveEdit() {
    if (editing === null) return;
    setEditError(null);
    setSaving(true);
    try {
      setLayout(
        editing.kind === "tag"
          ? await renameTag(editing.id, editingValue)
          : await renameTagGroup(editing.id, editingValue),
      );
      setEditing(null);
      onChanged();
    } catch (e) {
      setEditError(errorText(e));
    } finally {
      setSaving(false);
    }
  }

  async function handleConfirmDeleteGroup() {
    if (groupDeleteTarget === null) return;
    const id = groupDeleteTarget;
    setDeleting(true);
    setError(null);
    try {
      setLayout(await deleteTagGroup(id));
      if (editing?.kind === "group" && editing.id === id) setEditing(null);
      onChanged();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setDeleting(false);
      setGroupDeleteTarget(null);
    }
  }

  async function handleConfirmDelete() {
    if (!deleteTarget) return;
    const target = deleteTarget;
    setDeleting(true);
    setError(null);
    try {
      await deleteTag(target.id);
      if (editing?.kind === "tag" && editing.id === target.id) setEditing(null);
      // 削除は済んでいる。後続の取り直しが失敗しても、親には反映させる。
      onTagDeleted(target.id);
      onChanged();
      setLayout(await getTagLayout());
    } catch (e) {
      setError(errorText(e));
    } finally {
      setDeleting(false);
      setDeleteTarget(null);
    }
  }

  const lastIndex = layout.length - 1;

  // ConfirmDialog sits beside (not inside) the ModalShell; see ModalShell for why.
  return (
    <>
      <ModalShell
        label="タグの編集"
        className="tag-bar-edit-dialog"
        testId="tag-bar-edit-dialog"
        onDismiss={onClose}
        // 削除の確認が開いている間は、Escape で確認だけを閉じる。
        dismissible={
          !busy && deleteTarget === null && groupDeleteTarget === null
        }
      >
        <h2>タグの編集</h2>
        {creating && (
          <div className="tag-bar-edit-create" data-testid="tag-bar-edit-create">
            <div className="tag-bar-edit-input-controls">
              <input
                className="tag-bar-edit-name-input"
                data-testid="tag-bar-edit-create-input"
                aria-label="新しいグループ名"
                placeholder="グループ名"
                value={createValue}
                onChange={(e) => setCreateValue(e.target.value)}
                disabled={busy}
              />
              <button
                type="button"
                data-testid="tag-bar-edit-create-btn"
                onClick={handleCreate}
                disabled={busy}
              >
                作成
              </button>
              <button
                type="button"
                data-testid="tag-bar-edit-create-cancel-btn"
                onClick={() => setCreating(false)}
                disabled={busy}
              >
                キャンセル
              </button>
            </div>
            {createError && (
              <p className="tag-bar-edit-error" data-testid="tag-bar-edit-create-error">
                {createError}
              </p>
            )}
          </div>
        )}
        {layout.length === 0 ? (
          <p className="tag-bar-edit-empty">タグがありません。</p>
        ) : (
          <ul className="tag-bar-edit-list" data-testid="tag-bar-edit-list">
            {layout.map((item, index) => {
              if (item.kind === "tag") {
                return (
                  <TagRow
                    key={`tag-${item.id}`}
                    tag={item}
                    grouped={false}
                    disableUp={index === 0}
                    disableDown={index === lastIndex}
                    busy={busy}
                    editing={editing?.kind === "tag" && editing.id === item.id}
                    editingValue={editingValue}
                    editError={editError}
                    onMove={(direction) => handleMove("tag", item.id, direction)}
                    onStartEdit={() => startEdit("tag", item.id, item.name)}
                    onDelete={() => setDeleteTarget(item)}
                    onEditingValueChange={setEditingValue}
                    onSaveEdit={handleSaveEdit}
                    onCancelEdit={() => setEditing(null)}
                  />
                );
              }
              return (
                // 見出しと中のタグは、同じ平らな <ul> の兄弟の <li> にする(インデントはクラスで付ける)。
                <Fragment key={`group-${item.id}`}>
                    <TagGroupRow
                      name={item.name}
                      disableUp={index === 0}
                      disableDown={index === lastIndex}
                      busy={busy}
                      editing={editing?.kind === "group" && editing.id === item.id}
                      editingValue={editingValue}
                      editError={editError}
                      onMove={(direction) => handleMove("group", item.id, direction)}
                      onStartEdit={() => startEdit("group", item.id, item.name)}
                      onDelete={() => setGroupDeleteTarget(item.id)}
                      onEditingValueChange={setEditingValue}
                      onSaveEdit={handleSaveEdit}
                      onCancelEdit={() => setEditing(null)}
                    />
                    {/* グループの中のタグの ↑↓ は、端でもグループの外へ出られるので常に有効。 */}
                    {item.tags.map((tag) => (
                      <TagRow
                        key={`tag-${tag.id}`}
                        tag={tag}
                        grouped
                        disableUp={false}
                        disableDown={false}
                        busy={busy}
                        editing={editing?.kind === "tag" && editing.id === tag.id}
                        editingValue={editingValue}
                        editError={editError}
                        onMove={(direction) => handleMove("tag", tag.id, direction)}
                        onStartEdit={() => startEdit("tag", tag.id, tag.name)}
                        onDelete={() => setDeleteTarget(tag)}
                        onEditingValueChange={setEditingValue}
                        onSaveEdit={handleSaveEdit}
                        onCancelEdit={() => setEditing(null)}
                      />
                    ))}
                </Fragment>
              );
            })}
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
            data-testid="tag-bar-edit-add-group-btn"
            onClick={startCreate}
            disabled={busy}
          >
            + タググループを追加
          </button>
          <button
            type="button"
            data-testid="tag-bar-edit-close-btn"
            onClick={onClose}
            disabled={busy}
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
      <ConfirmDialog
        open={groupDeleteTarget !== null}
        title="タググループを削除"
        confirmLabel="削除する"
        danger
        busy={deleting}
        onConfirm={handleConfirmDeleteGroup}
        onCancel={() => setGroupDeleteTarget(null)}
      >
        <p>グループを削除します。中のタグはグループなしに戻ります。</p>
      </ConfirmDialog>
    </>
  );
}
