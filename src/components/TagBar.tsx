import { useEffect, useState } from "react";
import { listAllTags } from "../api";
import { onCatalogChanged } from "../events";
import type { TagDto } from "../types";

interface Props {
  selected: number[];
  onChange: (tagIds: number[]) => void;
  /** 編集モーダル保存後にparentがbumpする(FolderSidebarのrefreshKeyと同じ形) */
  refreshKey?: number;
}

// 左ペイン用のタグ絞り込み(複数選択)。全タグをフォルダ一覧と同じ縦のボタン
// リストで出す。順序は [すべて解除] の後に、編集画面で決めた並び
// (list_all_tags の順そのまま)。
// 並びの編集はネイティブメニュー「タグ > タグバーの編集...」からのみ開く
// (src-tauri/src/lib.rsのMENU_ITEM_TAG_BAR_MANAGE参照) -- この部品自体には
// 編集への導線は置かない。
export function TagBar({ selected, onChange, refreshKey }: Props) {
  const [allTags, setAllTags] = useState<TagDto[]>([]);

  // list_all_tags on mount + re-fetch on catalog:changed -- a newly assigned
  // tag (via a video's own TagEditor) must appear as a candidate here without
  // a full app reload. refreshKey is bumped by the parent once the edit
  // dialog saves a new order, which re-runs this effect to pick it up.
  useEffect(() => {
    let cancelled = false;
    const reload = () => {
      listAllTags()
        .then((tags) => {
          if (!cancelled) setAllTags(tags);
        })
        .catch(console.error);
    };
    reload();
    const unlisten = onCatalogChanged(reload);
    return () => {
      cancelled = true;
      unlisten();
    };
  }, [refreshKey]);

  if (allTags.length === 0) return null;

  function toggle(tagId: number) {
    onChange(
      selected.includes(tagId)
        ? selected.filter((id) => id !== tagId)
        : [...selected, tagId],
    );
  }

  return (
    <div className="tag-bar" data-testid="tag-bar">
      <button
        type="button"
        className="folder-sidebar-item tag-bar-clear-all"
        data-testid="tag-bar-clear-all"
        disabled={selected.length === 0}
        onClick={() => onChange([])}
      >
        すべて解除
      </button>
      {allTags.map((tag) => {
        const active = selected.includes(tag.id);
        return (
          <button
            key={tag.id}
            type="button"
            className={
              active
                ? "folder-sidebar-item folder-sidebar-item--active"
                : "folder-sidebar-item"
            }
            title={tag.name}
            onClick={() => toggle(tag.id)}
            aria-pressed={active}
            data-testid="tag-bar-chip"
          >
            {tag.name}
          </button>
        );
      })}
    </div>
  );
}
