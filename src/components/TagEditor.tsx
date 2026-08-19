import { useEffect, useState } from "react";
import type { FormEvent } from "react";
import { assignTag, listAllTags, listTagsForVideo, removeTag } from "../api";
import type { TagDto } from "../types";

interface Props {
  videoId: string;
}

// Tag name normalization (trim/full-width fold/empty rejection) happens
// exclusively in gb_core::tags on the backend -- this component never
// re-implements it, only forwards the raw input and surfaces whatever
// error the backend returns.
export function TagEditor({ videoId }: Props) {
  const [tags, setTags] = useState<TagDto[]>([]);
  // A plain suggestion list fed by list_all_tags (existing tags across the
  // whole catalog), not an incremental tag search/management screen.
  const [allTags, setAllTags] = useState<TagDto[]>([]);
  const [input, setInput] = useState("");
  const [error, setError] = useState<string | null>(null);

  async function reloadTags() {
    try {
      setTags(await listTagsForVideo(videoId));
    } catch (e) {
      console.error(e);
    }
  }

  useEffect(() => {
    // Inlined rather than calling the component-scope `reloadTags` directly
    // -- matches ThumbnailGrid's tick() idiom for async effect bodies.
    let cancelled = false;
    listTagsForVideo(videoId)
      .then((rows) => {
        if (!cancelled) setTags(rows);
      })
      .catch(console.error);
    listAllTags().then(setAllTags).catch(console.error);
    return () => {
      cancelled = true;
    };
  }, [videoId]);

  async function handleAdd(e: FormEvent) {
    e.preventDefault();
    if (!input.trim()) return;
    setError(null);
    try {
      await assignTag(videoId, input);
      setInput("");
      await reloadTags();
    } catch (e) {
      // Surfaced in the UI, not just console.error -- e.g. the video could
      // have gone offline/been removed between the grid loading and the
      // user tagging it here.
      setError(String(e));
    }
  }

  async function handleRemove(tagId: number) {
    setError(null);
    try {
      await removeTag(videoId, tagId);
      await reloadTags();
    } catch (e) {
      setError(String(e));
    }
  }

  return (
    <div className="tag-editor" data-testid="video-row-tags">
      <div className="tag-editor-chips">
        {tags.map((tag) => (
          <span key={tag.id} className="tag-chip">
            {tag.name}
            <button type="button" onClick={() => handleRemove(tag.id)} aria-label={`${tag.name}を削除`}>
              ×
            </button>
          </span>
        ))}
      </div>
      <form onSubmit={handleAdd}>
        <input
          type="text"
          value={input}
          onChange={(e) => setInput(e.target.value)}
          placeholder="タグを追加"
          list="tag-editor-suggestions"
        />
        <datalist id="tag-editor-suggestions">
          {allTags.map((tag) => (
            <option key={tag.id} value={tag.name} />
          ))}
        </datalist>
        <button type="submit">追加</button>
      </form>
      {error && <p className="tag-editor-error">{error}</p>}
    </div>
  );
}
