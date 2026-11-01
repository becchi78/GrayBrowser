// DOM-dependent (HTMLElement), so kept apart from the pure helpers that run
// under node's test runner.

// Keys typed into these must keep their native behavior (notably Ctrl+A as
// "select all text"), so list-level shortcuts ignore them. SELECT is included
// because it consumes keystrokes (type-ahead, arrows) too.
export function isTextEntryTarget(target: EventTarget): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return target.isContentEditable || ["INPUT", "TEXTAREA", "SELECT"].includes(target.tagName);
}

// Narrower than isTextEntryTarget: only elements whose native context menu
// offers cut/copy/paste. A SELECT has none of those, so it gets no exemption
// from the app-wide menu suppression. Read-only/disabled inputs stay exempt
// because Copy still works there.
export function isTextEditTarget(target: EventTarget): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return target.isContentEditable || ["INPUT", "TEXTAREA"].includes(target.tagName);
}
