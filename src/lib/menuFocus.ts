// Keyboard focus movement shared by the app's popup menus. Disabled items
// cannot take focus, so they are left out of both the initial focus and the
// arrow-key order.

// One attribute-prefix selector (menuitem / menuitemradio) rather than a selector
// list: some DOM implementations return a list's matches grouped per selector
// instead of in document order, which would scramble the arrow-key order.
export const ENABLED_ITEM_SELECTOR = '[role^="menuitem"]:not(:disabled)';

export type FocusMove = "next" | "prev" | "first" | "last";

/** The index to focus next; wraps at both ends. `current` is -1 when nothing in the menu has focus. */
export function nextFocusIndex(current: number, count: number, move: FocusMove): number {
  if (count === 0) return -1;
  switch (move) {
    case "first":
      return 0;
    case "last":
      return count - 1;
    case "next":
      return (current + 1) % count;
    case "prev":
      return current < 0 ? count - 1 : (current - 1 + count) % count;
  }
}

export function enabledItems(menu: HTMLElement | null): HTMLElement[] {
  return Array.from(menu?.querySelectorAll<HTMLElement>(ENABLED_ITEM_SELECTOR) ?? []);
}

export function focusFirstItem(menu: HTMLElement | null): void {
  enabledItems(menu)[0]?.focus();
}

export function moveMenuFocus(menu: HTMLElement | null, move: FocusMove): void {
  const items = enabledItems(menu);
  const current = items.indexOf(document.activeElement as HTMLElement);
  items[nextFocusIndex(current, items.length, move)]?.focus();
}
