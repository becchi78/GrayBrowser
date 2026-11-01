// DOM-dependent (HTMLElement), so kept apart from the pure helpers that run
// under node's test runner.

// Keys typed into these must keep their native behavior (notably Ctrl+A as
// "select all text"), so list-level shortcuts ignore them. SELECT is included
// because it consumes keystrokes (type-ahead, arrows) too.
export function isTextEntryTarget(target: EventTarget): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return target.isContentEditable || ["INPUT", "TEXTAREA", "SELECT"].includes(target.tagName);
}

const TEXT_LIKE_INPUT_TYPES = ["text", "search", "url", "tel", "email", "password", "number"];

// Narrower than isTextEntryTarget: only elements where cut/copy/paste is
// meaningful, i.e. the ones that get the app's edit menu. Checkboxes, buttons,
// range inputs and SELECT have nothing to cut or paste. Read-only/disabled
// fields stay included because Copy still works there.
export function isTextEditTarget(target: EventTarget): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target.isContentEditable || target.tagName === "TEXTAREA") return true;
  if (target.tagName !== "INPUT") return false;
  // The `type` property normalizes a missing/unknown attribute to "text".
  return TEXT_LIKE_INPUT_TYPES.includes((target as HTMLInputElement).type);
}
