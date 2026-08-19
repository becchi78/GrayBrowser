import { useEffect, useState } from "react";

interface Props {
  value: string;
  onChange: (value: string) => void;
}

const DEBOUNCE_MS = 200;

// Forwards the raw, unparsed search string upward after a trailing-edge
// debounce -- term-splitting/substring-matching logic lives entirely in
// gb_core::search + the backend query layer (list_videos_filtered), never
// duplicated here.
export function SearchBox({ value, onChange }: Props) {
  const [draft, setDraft] = useState(value);

  useEffect(() => {
    const timer = setTimeout(() => {
      if (draft !== value) onChange(draft);
    }, DEBOUNCE_MS);
    return () => clearTimeout(timer);
    // Only re-debounce when the user's own typing (draft) changes -- not
    // when `value`/`onChange` change as a side effect of that same commit.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [draft]);

  return (
    <input
      type="text"
      className="search-box"
      value={draft}
      onChange={(e) => setDraft(e.target.value)}
      placeholder="検索..."
      aria-label="検索"
      data-testid="header-search-input"
    />
  );
}
