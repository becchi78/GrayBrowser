import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { getTagBarPinnedTagIds, listAllTags } from "../api";
import { onCatalogChanged } from "../events";
import { computeVisibleChipCount } from "../lib/tagBarLayout";
import type { TagDto } from "../types";

interface Props {
  selected: number[];
  onChange: (tagIds: number[]) => void;
  /** 編集モーダル保存後にparentがbumpする(FolderSidebarのrefreshKeyと同じ形。次フェーズで実際に使われる) */
  refreshKey?: number;
  onOpenEditDialog: () => void;
}

// TagFilter.tsx（全タグを折り返し表示するフィルタ行）の後継。表示順序は
// 固定: [すべて解除] [promoted...] [pinned...] [▾] [編集 ▸]。
// promotedTagIds（選択したが未pinのタグ）専用stateは持たず、selectedと
// pinnedTagIdsから毎レンダー導出する -- selectedから外れれば自動的に
// promotedからも消えるので、二重管理・同期漏れの余地がない。
export function TagBar({ selected, onChange, refreshKey, onOpenEditDialog }: Props) {
  const [allTags, setAllTags] = useState<TagDto[]>([]);
  const [pinnedTagIds, setPinnedTagIds] = useState<number[]>([]);
  const [windowInnerWidth, setWindowInnerWidth] = useState(() => window.innerWidth);
  const [chipWidthsPx, setChipWidthsPx] = useState<number[]>([]);
  const [clearAllWidthPx, setClearAllWidthPx] = useState(0);
  const [editLinkWidthPx, setEditLinkWidthPx] = useState(0);
  const [overflowButtonWidthPx, setOverflowButtonWidthPx] = useState(0);
  const [overflowOpen, setOverflowOpen] = useState(false);

  const clearAllRef = useRef<HTMLButtonElement>(null);
  const editLinkRef = useRef<HTMLButtonElement>(null);
  const overflowProbeRef = useRef<HTMLButtonElement>(null);
  const measureRowRef = useRef<HTMLDivElement>(null);

  // Same "list_all_tags on mount + re-fetch on catalog:changed" pattern
  // TagFilter.tsx used -- a newly assigned tag (via a video's own TagEditor)
  // must appear as a candidate here without a full app reload.
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

  // Same "re-fetch on refreshKey change" pattern as FolderSidebar's own
  // list_watch_folders effect -- refreshKey is bumped by the parent once the
  // (next-phase) edit dialog saves a new pinned list.
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

  // Only the window width is recomputed on resize -- chip widths themselves
  // don't change just because the window resized, so re-measuring them here
  // too would be wasted work (same responsibility split as FolderSidebar's
  // own `handleResize`, which also only touches the one value the resize
  // itself can actually affect).
  useEffect(() => {
    function handleResize() {
      setWindowInnerWidth(window.innerWidth);
    }
    window.addEventListener("resize", handleResize);
    return () => window.removeEventListener("resize", handleResize);
  }, []);

  const tagsById = useMemo(() => new Map(allTags.map((tag) => [tag.id, tag])), [allTags]);
  const pinnedSet = useMemo(() => new Set(pinnedTagIds), [pinnedTagIds]);

  // selected is already in click order, so this is the entire "promotion"
  // mechanism: a just-clicked, not-yet-pinned tag appears here (right after
  // the other promoted tags, before the pinned ones) the moment it's added
  // to selected, and disappears the moment it's removed -- no separate
  // "promoted" state to keep in sync.
  const promotedTagIds = useMemo(
    () => selected.filter((id) => !pinnedSet.has(id)),
    [selected, pinnedSet],
  );

  // Ids of a tag that's been deleted (via TagEditor's delete flow elsewhere)
  // can still linger in `selected`/pinnedTagIds for one render until their
  // own self-healing effects catch up -- filtered out here so this
  // component never tries to render a chip with no matching TagDto.
  const candidateIds = useMemo(
    () => [...promotedTagIds, ...pinnedTagIds].filter((id) => tagsById.has(id)),
    [promotedTagIds, pinnedTagIds, tagsById],
  );

  // Re-measures whenever the candidate chip set (identity + order) changes,
  // or once the library goes from having no tags at all to having some
  // (nothing is mounted below to measure while `allTags.length === 0`, since
  // this component renders `null` in that case -- see the bottom of this
  // function). A plain `useEffect` would measure *after* paint, letting an
  // over-budget layout flash for one frame; `useLayoutEffect` measures and
  // re-renders before the browser paints instead.
  useLayoutEffect(() => {
    if (allTags.length === 0) return;
    const row = measureRowRef.current;
    if (row) {
      setChipWidthsPx(
        Array.from(row.children).map((el) => (el as HTMLElement).getBoundingClientRect().width),
      );
    }
    if (clearAllRef.current) {
      setClearAllWidthPx(clearAllRef.current.getBoundingClientRect().width);
    }
    if (editLinkRef.current) {
      setEditLinkWidthPx(editLinkRef.current.getBoundingClientRect().width);
    }
    if (overflowProbeRef.current) {
      setOverflowButtonWidthPx(overflowProbeRef.current.getBoundingClientRect().width);
    }
  }, [candidateIds, allTags.length]);

  if (allTags.length === 0) return null;

  function toggle(tagId: number) {
    onChange(
      selected.includes(tagId) ? selected.filter((id) => id !== tagId) : [...selected, tagId],
    );
  }

  // `computeVisibleChipCount`'s `clearAllWidthPx` parameter really just
  // means "reserve this much space before the chips" -- folding the
  // always-visible "編集 ▸" link's own width into it here reserves space for
  // both always-present controls without changing that pure function's
  // signature.
  const visibleCount = computeVisibleChipCount(
    windowInnerWidth,
    clearAllWidthPx + editLinkWidthPx,
    overflowButtonWidthPx,
    chipWidthsPx,
  );
  const displayedIds = candidateIds.slice(0, visibleCount);
  const displayedSet = new Set(displayedIds);
  // The ▾ dropdown's job is broader than just "candidates that didn't fit":
  // it's the only remaining way to reach a tag that isn't currently
  // pinned/selected at all (TagFilter.tsx used to just render every tag as
  // a wrapped chip; this bar only ever shows promoted/pinned chips inline).
  const overflowTags = allTags.filter((tag) => !displayedSet.has(tag.id));

  return (
    <div className="tag-bar" data-testid="tag-bar">
      <button
        type="button"
        className="tag-bar-clear-all"
        data-testid="tag-bar-clear-all"
        ref={clearAllRef}
        disabled={selected.length === 0}
        onClick={() => onChange([])}
      >
        すべて解除
      </button>
      {displayedIds.map((id) => {
        const tag = tagsById.get(id);
        if (!tag) return null;
        return (
          <button
            key={tag.id}
            type="button"
            className={
              selected.includes(tag.id) ? "tag-bar-chip tag-bar-chip--active" : "tag-bar-chip"
            }
            title={tag.name}
            onClick={() => toggle(tag.id)}
            aria-pressed={selected.includes(tag.id)}
            data-testid="tag-bar-chip"
          >
            {tag.name}
          </button>
        );
      })}
      {overflowTags.length > 0 && (
        <div className="tag-bar-overflow">
          <button
            type="button"
            className="tag-bar-overflow-toggle"
            data-testid="tag-bar-overflow-toggle"
            aria-expanded={overflowOpen}
            onClick={() => setOverflowOpen((open) => !open)}
          >
            ▾
          </button>
          {overflowOpen && (
            <div className="tag-bar-overflow-panel" data-testid="tag-bar-overflow-panel">
              <div className="tag-bar-overflow-list">
                {overflowTags.map((tag) => (
                  <button
                    key={tag.id}
                    type="button"
                    className={
                      selected.includes(tag.id)
                        ? "tag-bar-chip tag-bar-chip--active"
                        : "tag-bar-chip"
                    }
                    title={tag.name}
                    onClick={() => {
                      toggle(tag.id);
                      setOverflowOpen(false);
                    }}
                    aria-pressed={selected.includes(tag.id)}
                    data-testid="tag-bar-overflow-chip"
                  >
                    {tag.name}
                  </button>
                ))}
              </div>
            </div>
          )}
        </div>
      )}
      <button
        type="button"
        className="tag-bar-edit-link"
        data-testid="tag-bar-edit-link"
        ref={editLinkRef}
        onClick={onOpenEditDialog}
      >
        編集 ▸
      </button>

      {/* Hidden measurement row: real markup (same classes as the visible
          chips above, so `max-width`/ellipsis clamping is reflected in the
          measured width too), rendered off-screen so `useLayoutEffect` can
          read each element's real `getBoundingClientRect().width` before
          paint. `visibility: hidden` (not `display: none`) keeps it in the
          layout tree so it still has a real, measurable size. */}
      <div
        aria-hidden="true"
        style={{ position: "absolute", top: 0, left: 0, visibility: "hidden", pointerEvents: "none" }}
      >
        <button type="button" className="tag-bar-overflow-toggle" ref={overflowProbeRef}>
          ▾
        </button>
        <div ref={measureRowRef} style={{ display: "flex", whiteSpace: "nowrap" }}>
          {candidateIds.map((id) => {
            const tag = tagsById.get(id);
            return tag ? (
              <button key={id} type="button" className="tag-bar-chip">
                {tag.name}
              </button>
            ) : null;
          })}
        </div>
      </div>
    </div>
  );
}
