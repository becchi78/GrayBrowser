import { useEffect, useState } from "react";
import { listAllTags } from "../api";
import { onCatalogChanged } from "../events";
import type { TagDto } from "../types";

interface Props {
  selected: number[];
  onChange: (tagIds: number[]) => void;
}

// A simple toggleable chip list fed by list_all_tags: AND-only filtering,
// no query syntax/OR/NOT/hierarchy. Tag rename/merge/delete stays in
// VideoRow's TagEditor, out of scope here.
export function TagFilter({ selected, onChange }: Props) {
  const [allTags, setAllTags] = useState<TagDto[]>([]);

  // Originally fetched once on mount only, so a brand-new tag assigned via
  // a video's own TagEditor never appeared here as a filter chip until the
  // next full app reload.
  // `assign_tag`/`remove_tag` (tag_cmds.rs) already fire `catalog:changed`
  // on success -- the same event `ThumbnailGrid` already re-fetches videos
  // on (see that component's own `onCatalogChanged` subscription for the
  // unlisten/cancelled-guard pattern this mirrors) -- so subscribing here
  // too is enough; no backend change needed. Deliberately un-throttled
  // (unlike ThumbnailGrid's NOTIFY_THROTTLE_MS): list_all_tags is a light,
  // single-table read (no thumbnails/joins), and re-running it once per
  // catalog:changed event (which is itself already coalesced to real
  // mutations, not a high-frequency stream) is cheap enough not to need a
  // throttle of its own.
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

  if (allTags.length === 0) return null;

  function toggle(tagId: number) {
    onChange(
      selected.includes(tagId) ? selected.filter((id) => id !== tagId) : [...selected, tagId],
    );
  }

  return (
    <div className="tag-filter">
      {allTags.map((tag) => (
        <button
          key={tag.id}
          type="button"
          className={
            selected.includes(tag.id) ? "tag-filter-chip tag-filter-chip--active" : "tag-filter-chip"
          }
          onClick={() => toggle(tag.id)}
          aria-pressed={selected.includes(tag.id)}
          data-testid="filter-chip"
        >
          {tag.name}
        </button>
      ))}
    </div>
  );
}
