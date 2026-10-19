// Pure selection logic behind the video list's multi-select. Split out of
// ThumbnailGrid so the click/range rules are unit-testable without mounting
// it. Selection is id-based (not index-based) so rows scrolled out of the
// virtualizer's window stay selected. Every function returns a new state
// object and never mutates its input, matching how React state setters use
// it.

export interface SelectionState {
  ids: ReadonlySet<string>;
  /** Range origin for Shift+click: the row last clicked without Shift. */
  anchorId: string | null;
}

export interface ClickModifiers {
  ctrl: boolean;
  shift: boolean;
}

export function clearSelection(): SelectionState {
  return { ids: new Set(), anchorId: null };
}

export function selectAll(orderedIds: readonly string[]): SelectionState {
  return { ids: new Set(orderedIds), anchorId: null };
}

/**
 * Plain click selects only the clicked row; Ctrl toggles it; Shift selects
 * the range from the anchor (Ctrl+Shift adds that range to the current
 * selection). Shift keeps the anchor so repeated Shift+clicks pivot around
 * the same origin, like a file manager. Shift with no usable anchor (none, or
 * no longer in the list) falls back to a plain/Ctrl click, since there is no
 * range to take.
 */
export function clickSelect(
  state: SelectionState,
  orderedIds: readonly string[],
  clickedId: string,
  { ctrl, shift }: ClickModifiers,
): SelectionState {
  const clickedIndex = orderedIds.indexOf(clickedId);
  const anchorIndex = state.anchorId === null ? -1 : orderedIds.indexOf(state.anchorId);

  if (shift && clickedIndex !== -1 && anchorIndex !== -1) {
    const from = Math.min(anchorIndex, clickedIndex);
    const to = Math.max(anchorIndex, clickedIndex);
    const range = orderedIds.slice(from, to + 1);
    const ids = ctrl ? new Set([...state.ids, ...range]) : new Set(range);
    return { ids, anchorId: state.anchorId };
  }

  if (ctrl) {
    const ids = new Set(state.ids);
    if (!ids.delete(clickedId)) ids.add(clickedId);
    return { ids, anchorId: clickedId };
  }

  return { ids: new Set([clickedId]), anchorId: clickedId };
}

/** Drops ids (and the anchor) that are no longer in the list. */
export function pruneSelection(
  state: SelectionState,
  orderedIds: readonly string[],
): SelectionState {
  const present = new Set(orderedIds);
  return {
    ids: new Set([...state.ids].filter((id) => present.has(id))),
    anchorId: state.anchorId !== null && present.has(state.anchorId) ? state.anchorId : null,
  };
}
