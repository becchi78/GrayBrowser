import { useEffect, useState } from "react";
import { listAllTags } from "../api";
import { onCatalogChanged } from "../events";
import { buildTagTree } from "../lib/tagTree";
import type { TagDto } from "../types";

interface Props {
  selected: number[];
  onChange: (tagIds: number[]) => void;
  /** 編集モーダル保存後にparentがbumpする(FolderSidebarのrefreshKeyと同じ形) */
  refreshKey?: number;
}

// 左ペイン用のタグ絞り込み(複数選択)。全タグをフォルダ一覧と同じ縦のボタン
// リストで出す。順序は [すべて解除] の後に、編集画面で決めた並び。子を持つ
// タグ(親)は開閉ボタン+絞り込みボタンの行で、その下に子を一段深く字下げして
// 並べる。親を押すと親の選択だけが切り替わる(子は選択状態にならない。親+子へ
// の拡張はバックエンドの絞り込みが行う)。
// 並びの編集はネイティブメニュー「タグ > タグの編集...」からのみ開く
// (src-tauri/src/lib.rsのMENU_ITEM_TAG_BAR_MANAGE参照) -- この部品自体には
// 編集への導線は置かない。
export function TagBar({ selected, onChange, refreshKey }: Props) {
  const [allTags, setAllTags] = useState<TagDto[]>([]);
  // 折りたたんだ親。画面内の状態で、再起動で展開に戻る。
  const [collapsed, setCollapsed] = useState<Set<number>>(new Set());

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

  function toggleCollapsed(tagId: number) {
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (!next.delete(tagId)) next.add(tagId);
      return next;
    });
  }

  function chip(tag: TagDto, extraClass: string) {
    const active = selected.includes(tag.id);
    return (
      <button
        key={tag.id}
        type="button"
        className={
          (active
            ? "folder-sidebar-item folder-sidebar-item--active"
            : "folder-sidebar-item") + extraClass
        }
        title={tag.name}
        onClick={() => toggle(tag.id)}
        aria-pressed={active}
        data-testid="tag-bar-chip"
      >
        {tag.name}
      </button>
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
      {buildTagTree(allTags).map(({ tag, children }) => {
        if (children.length === 0) return chip(tag, "");
        const isCollapsed = collapsed.has(tag.id);
        // 折りたたんでいても、選択中の子は見えるままにする(選択中であることが
        // 隠れて、絞り込みの理由が分からなくならないように)。
        const visibleChildren = isCollapsed
          ? children.filter((c) => selected.includes(c.id))
          : children;
        const active = selected.includes(tag.id);
        return (
          <div key={tag.id} className="tag-bar-group">
            <div
              className={
                active ? "tag-bar-parent-row tag-bar-parent-row--active" : "tag-bar-parent-row"
              }
            >
              <button
                type="button"
                className="tag-bar-toggle"
                aria-expanded={!isCollapsed}
                aria-label={`${tag.name}の子のタグを${isCollapsed ? "表示" : "隠す"}`}
                data-testid="tag-bar-toggle"
                onClick={() => toggleCollapsed(tag.id)}
              >
                {isCollapsed ? "▸" : "▾"}
              </button>
              <button
                type="button"
                className="folder-sidebar-item tag-bar-parent-chip"
                title={tag.name}
                onClick={() => toggle(tag.id)}
                aria-pressed={active}
                data-testid="tag-bar-chip"
              >
                {tag.name}
              </button>
            </div>
            {visibleChildren.map((child) => chip(child, " tag-bar-child"))}
          </div>
        );
      })}
    </div>
  );
}
