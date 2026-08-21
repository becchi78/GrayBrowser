// Pure array operations behind TagBarEditDialog's "バーに常設するタグ" list
// (↑/↓ reorder, ✕ remove, + 追加) -- split out into this JSX非依存 file so
// they're unit-testable without mounting the dialog, same pattern as
// src/lib/sidebarResize.ts/tagBarLayout.ts. Every function returns a new
// array (never mutates `ids`), matching how React state setters are used
// at each call site (`setDraftPinnedIds((ids) => moveUp(ids, index))`).

/**
 * Swaps the element at `index` with the one before it. A no-op (returns
 * `ids` unchanged, same reference) if `index` is already the first element
 * or out of range -- callers also disable the "↑" button in that case, but
 * this stays safe to call regardless.
 */
export function moveUp(ids: number[], index: number): number[] {
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
export function moveDown(ids: number[], index: number): number[] {
  return moveUp(ids, index + 1);
}

/** Removes the element at `index`. A no-op for an out-of-range index. */
export function removeAt(ids: number[], index: number): number[] {
  if (index < 0 || index >= ids.length) return ids;
  return ids.filter((_, i) => i !== index);
}

/**
 * Appends `id` to the end, unless it's already present -- the "その他の
 * タグ"側の「+ 追加」ボタンはUI側で既にdraftPinnedIdsに含まれるものを
 * 除外して表示しているため通常は重複しないが、二重クリック等に対する
 * 保険として重複追加を防ぐ。
 */
export function addIfAbsent(ids: number[], id: number): number[] {
  return ids.includes(id) ? ids : [...ids, id];
}
