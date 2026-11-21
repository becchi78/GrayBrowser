import { useEffect, useRef, useState } from "react";
import { getCollapsedTagGroups, getTagLayout, setCollapsedTagGroups } from "../api";
import { onCatalogChanged } from "../events";
import type { TagLayoutItemDto, TagWithCountDto } from "../types";
import { CountPill } from "./CountPill";
import { ChevronRightIcon, TagGroupIcon } from "./SidebarIcons";

interface Props {
  selected: number[];
  onChange: (tagIds: number[]) => void;
  /** 編集モーダル保存後にparentがbumpする(FolderSidebarのrefreshKeyと同じ形) */
  refreshKey?: number;
  /** 各タグの件数のピルを出すか。 */
  showCounts?: boolean;
}

// 左ペイン用のタグ絞り込み(複数選択)。タグとタググループをフォルダ一覧と同じ縦の
// ボタンリストで出す。順序は [すべて解除] の後に、編集画面で決めた並び
// (バックエンドが返す順そのまま)。
// グループは表示上の入れ物で、クリックは開閉だけ。絞り込みは選択中のタグ
// そのものの AND のままで、グループの有無で意味は変わらない。
// 並びの編集は、ハンバーガーメニューの「タグの編集...」からのみ開く。
// この部品自体には編集への導線は置かない。
export function TagBar({ selected, onChange, refreshKey, showCounts = false }: Props) {
  const [layout, setLayout] = useState<TagLayoutItemDto[]>([]);
  const [collapsed, setCollapsed] = useState<number[]>([]);
  // 読み込み前にユーザーが開閉したら、あとから届く保存済みの状態で上書きしない
  // (上書きすると、表示が直前の保存内容と食い違う)。
  const toggledRef = useRef(false);

  // get_tag_layout on mount + re-fetch on catalog:changed -- a newly assigned
  // tag (via a video's own TagEditor) must appear as a candidate here without
  // a full app reload. refreshKey is bumped by the parent once the edit
  // dialog saves a new order, which re-runs this effect to pick it up.
  useEffect(() => {
    let cancelled = false;
    const reload = () => {
      getTagLayout()
        .then((items) => {
          if (!cancelled) setLayout(items);
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

  // 折りたたみ状態は起動時に1回だけ読む(未保存なら全部開く)。
  useEffect(() => {
    let cancelled = false;
    getCollapsedTagGroups()
      .then((ids) => {
        if (!cancelled && !toggledRef.current) setCollapsed(ids);
      })
      .catch(console.error);
    return () => {
      cancelled = true;
    };
  }, []);

  if (layout.length === 0) return null;

  function toggle(tagId: number) {
    onChange(
      selected.includes(tagId) ? selected.filter((id) => id !== tagId) : [...selected, tagId],
    );
  }

  function toggleGroup(groupId: number) {
    const existing = new Set<number>();
    for (const item of layout) {
      if (item.kind === "group") existing.add(item.id);
    }
    const toggled = collapsed.includes(groupId)
      ? collapsed.filter((id) => id !== groupId)
      : [...collapsed, groupId];
    // 削除済みのグループの id は、保存のたびに落とす。残すと状態が溜まり続ける。
    const next = toggled.filter((id) => existing.has(id));
    // Optimistic: the UI flips immediately; a failed save only costs
    // persistence across restarts, so it is logged rather than reverted.
    toggledRef.current = true;
    setCollapsed(next);
    setCollapsedTagGroups(next).catch((e) => console.error(e));
  }

  function renderTag(tag: TagWithCountDto, grouped: boolean) {
    const active = selected.includes(tag.id);
    let className = "folder-sidebar-item";
    if (active) className += " folder-sidebar-item--active";
    if (grouped) className += " tag-bar-chip--grouped";
    return (
      <button
        key={`tag-${tag.id}`}
        type="button"
        className={className}
        title={tag.name}
        onClick={() => toggle(tag.id)}
        aria-pressed={active}
        data-testid="tag-bar-chip"
      >
        {tag.name}
        {showCounts && <CountPill count={tag.video_count} />}
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
      {layout.map((item) => {
        if (item.kind === "tag") return renderTag(item, false);
        const open = !collapsed.includes(item.id);
        const selectedCount = item.tags.filter((t) => selected.includes(t.id)).length;
        return (
          <div key={`group-${item.id}`} className="tag-bar-group-block">
            <button
              type="button"
              className="folder-sidebar-item tag-bar-group"
              title={item.name}
              aria-expanded={open}
              onClick={() => toggleGroup(item.id)}
              data-testid="tag-bar-group"
            >
              <ChevronRightIcon className="tag-bar-group-chevron" />
              <TagGroupIcon className="tag-bar-group-icon" />
              <span className="tag-bar-group-name">{item.name}</span>
              {!open && selectedCount > 0 && (
                <span className="tag-bar-group-badge" data-testid="tag-bar-group-badge">
                  {selectedCount}
                </span>
              )}
            </button>
            {open && item.tags.map((tag) => renderTag(tag, true))}
          </div>
        );
      })}
    </div>
  );
}
