// Pure ↑/↓ reorder operations behind TagBarEditDialog's tag list and
// FolderDialog's folder list. Generic over the element type so both can share
// them (tag objects, string paths). Split out into this JSX非依存 file so
// they're unit-testable without mounting the dialog, same pattern as
// src/lib/sidebarResize.ts. Every function returns a new
// array (never mutates `ids`), matching how React state setters are used
// at each call site (`setDraftTags((tags) => moveUp(tags, index))`).

/**
 * Swaps the element at `index` with the one before it. A no-op (returns
 * `ids` unchanged, same reference) if `index` is already the first element
 * or out of range -- callers also disable the "↑" button in that case, but
 * this stays safe to call regardless.
 */
export function moveUp<T>(ids: T[], index: number): T[] {
  if (index <= 0 || index >= ids.length) return ids;
  const next = [...ids];
  [next[index - 1], next[index]] = [next[index], next[index - 1]];
  return next;
}

/**
 * Swaps the element at `index` with the one after it. Implemented as
 * `moveUp` on the next index, so the two share one boundary/swap
 * implementation instead of two independent (and independently buggy)
 * copies.
 */
export function moveDown<T>(ids: T[], index: number): T[] {
  return moveUp(ids, index + 1);
}
