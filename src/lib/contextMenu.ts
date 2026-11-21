// Pure positioning for the right-click menu, split out so the edge handling
// is unit-testable without a DOM.

export interface MenuPosition {
  x: number;
  y: number;
}

export type ContextMenuAction = "native" | "edit-menu" | "suppress";

/**
 * What a right-click should do. The WebView's built-in menu (Back/Reload,
 * and in text fields extras like password/emoji entries) is meaningless in a
 * desktop app, so text fields get the app's own cut/copy/paste menu instead.
 * Dev builds keep the native menu so Inspect stays reachable.
 */
export function contextMenuAction(opts: {
  isTextEdit: boolean;
  isDev: boolean;
}): ContextMenuAction {
  if (opts.isDev) return "native";
  return opts.isTextEdit ? "edit-menu" : "suppress";
}

/**
 * Keeps a menu of the given size fully inside the viewport. The menu opens at
 * the pointer; when it would overflow the right/bottom edge it is shifted back
 * (not flipped, so it never jumps far from the pointer). A menu larger than
 * the viewport is pinned to the top-left margin so its first item stays
 * reachable.
 */
export function clampMenuPosition(
  pointer: MenuPosition,
  menu: { width: number; height: number },
  viewport: { width: number; height: number },
  margin = 4,
): MenuPosition {
  const maxX = viewport.width - menu.width - margin;
  const maxY = viewport.height - menu.height - margin;
  return {
    x: Math.max(margin, Math.min(pointer.x, maxX)),
    y: Math.max(margin, Math.min(pointer.y, maxY)),
  };
}

/**
 * Where a dropdown opens so that its right edge lines up with the anchor's
 * right edge, directly below the anchor. Shifted inward when it would overflow.
 */
export function placeDropdownRightAligned(
  anchor: { right: number; bottom: number },
  menu: { width: number; height: number },
  viewport: { width: number; height: number },
  gap = 4,
  margin = 4,
): MenuPosition {
  return clampMenuPosition(
    { x: anchor.right - menu.width, y: anchor.bottom + gap },
    menu,
    viewport,
    margin,
  );
}

export interface SubmenuPlacement extends MenuPosition {
  side: "left" | "right";
}

/**
 * Where a submenu opens beside its parent menu. The parent sits near the
 * right edge of the window, so the left side is preferred; the right side is
 * used when the left would run off the window. The submenu is flush against
 * the parent's edge; only when neither side has room (a window far narrower
 * than the minimum) does the right-side submenu slide left over the parent so
 * that it stays inside the window. `itemTop` is the top of the parent item, to
 * which the submenu's first row is aligned.
 */
export function placeSubmenu(
  parent: { left: number; right: number },
  itemTop: number,
  menu: { width: number; height: number },
  viewport: { width: number; height: number },
  margin = 4,
): SubmenuPlacement {
  const leftX = parent.left - menu.width;
  const side = leftX >= margin ? "left" : "right";
  const x = side === "left" ? leftX : parent.right;
  const clamped = clampMenuPosition({ x, y: itemTop }, menu, viewport, margin);
  // On the left, x stays flush against the parent and only y is clamped. On the
  // right, x is pulled back inside the window, which can overlap the parent.
  return { x: side === "left" ? x : Math.min(x, clamped.x), y: clamped.y, side };
}
