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
