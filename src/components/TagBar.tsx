import { useEffect, useMemo, useState } from "react";
import { getTagBarPinnedTagIds, listAllTags } from "../api";
import { onCatalogChanged } from "../events";
import { deriveTagLists } from "../lib/sidebarSections";
import type { TagDto } from "../types";

interface Props {
  selected: number[];
  onChange: (tagIds: number[]) => void;
  /** 編集モーダル保存後にparentがbumpする(FolderSidebarのrefreshKeyと同じ形) */
  refreshKey?: number;
}

// 左ペイン用のタグ絞り込み(複数選択)。表示順序は固定:
// [すべて解除] [昇格(選択中の未常設)...] [常設...] [その他のタグ ▸]。
// 常設タグの編集はネイティブメニュー「タグ > タグバーの編集...」からのみ開く
// (src-tauri/src/lib.rsのMENU_ITEM_TAG_BAR_MANAGE参照) -- この部品自体には
// 編集への導線は置かない。
// 昇格用のstateは持たず、selectedとpinnedTagIdsから毎レンダー導出する
// (deriveTagLists) -- selectedから外れれば自動的に昇格も消えるので、
// 二重管理・同期漏れの余地がない。
export function TagBar({ selected, onChange, refreshKey }: Props) {
  const [allTags, setAllTags] = useState<TagDto[]>([]);
  const [pinnedTagIds, setPinnedTagIds] = useState<number[]>([]);
  const [othersOpen, setOthersOpen] = useState(false);

  // list_all_tags on mount + re-fetch on catalog:changed -- a newly assigned
  // tag (via a video's own TagEditor) must appear as a candidate here without
  // a full app reload.
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
  }, []);

  // refreshKey is bumped by the parent once the edit dialog saves a new
  // pinned list.
  useEffect(() => {
    let cancelled = false;
    getTagBarPinnedTagIds()
      .then((ids) => {
        if (!cancelled) setPinnedTagIds(ids);
      })
      .catch(console.error);
    return () => {
      cancelled = true;
    };
  }, [refreshKey]);

  const { primary, others } = useMemo(
    () => deriveTagLists(allTags, pinnedTagIds, selected),
    [allTags, pinnedTagIds, selected],
  );

  if (allTags.length === 0) return null;

  function toggle(tagId: number) {
    onChange(
      selected.includes(tagId) ? selected.filter((id) => id !== tagId) : [...selected, tagId],
    );
  }

  function renderChip(tag: TagDto, testId: string) {
    const active = selected.includes(tag.id);
    return (
      <button
        key={tag.id}
        type="button"
        className={active ? "tag-bar-chip tag-bar-chip--active" : "tag-bar-chip"}
        title={tag.name}
        onClick={() => toggle(tag.id)}
        aria-pressed={active}
        data-testid={testId}
      >
        {tag.name}
      </button>
    );
  }

  return (
    <div className="tag-bar" data-testid="tag-bar">
      <button
        type="button"
        className="tag-bar-clear-all"
        data-testid="tag-bar-clear-all"
        disabled={selected.length === 0}
        onClick={() => onChange([])}
      >
        すべて解除
      </button>
      <div className="tag-bar-chips">{primary.map((tag) => renderChip(tag, "tag-bar-chip"))}</div>
      {others.length > 0 && (
        <div className="tag-bar-others">
          <button
            type="button"
            className="tag-bar-others-toggle"
            data-testid="tag-bar-others-toggle"
            aria-expanded={othersOpen}
            onClick={() => setOthersOpen((open) => !open)}
          >
            <span aria-hidden="true">{othersOpen ? "▾" : "▸"}</span> その他のタグ
          </button>
          {othersOpen && (
            <div className="tag-bar-chips" data-testid="tag-bar-others-list">
              {others.map((tag) => renderChip(tag, "tag-bar-others-chip"))}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
