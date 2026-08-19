// Thin wrapper around the backend's events -- mirrors api.ts's role for
// invoke(): the event name and tauri-apps/api/event glue are only dealt
// with here.

import { listen } from "@tauri-apps/api/event";
import type { DuplicateGroup, WbImportProgress, WbImportSummary } from "./types";

/**
 * Subscribes to a Tauri event and returns an unlisten function. Safe to
 * call even if the listener hasn't attached yet by the time the caller
 * wants to unsubscribe (e.g. a component unmounting immediately after
 * mount): `listen()` is async, so if the caller unsubscribes before it
 * resolves, `cancelled` is set and the listener is torn down immediately
 * once it does attach, instead of leaking a live subscription.
 */
function subscribe<T>(eventName: string, handler: (payload: T) => void): () => void {
  let unlisten: (() => void) | undefined;
  let cancelled = false;

  listen<T>(eventName, (event) => handler(event.payload)).then((fn) => {
    if (cancelled) {
      fn();
    } else {
      unlisten = fn;
    }
  });

  return () => {
    cancelled = true;
    unlisten?.();
  };
}

/**
 * Subscribes to catalog:changed.
 */
export function onCatalogChanged(cb: () => void): () => void {
  return subscribe<void>("catalog:changed", cb);
}

/**
 * Subscribes to wb_import:progress (the `.wb` import pipeline).
 */
export function onWbImportProgress(cb: (progress: WbImportProgress) => void): () => void {
  return subscribe<WbImportProgress>("wb_import:progress", cb);
}

/**
 * Subscribes to wb_import:complete (the `.wb` import pipeline).
 */
export function onWbImportComplete(cb: (summary: WbImportSummary) => void): () => void {
  return subscribe<WbImportSummary>("wb_import:complete", cb);
}

/**
 * Subscribes to wb_import:failed (the `.wb` import pipeline). Payload is
 * a human-readable reason string.
 */
export function onWbImportFailed(cb: (reason: string) => void): () => void {
  return subscribe<string>("wb_import:failed", cb);
}

/**
 * Subscribes to dedup:updated (duplicate detection). Fired once
 * `dedup::refresh_duplicate_groups`'s background pass completes; the
 * payload is the full, freshly detected group list.
 */
export function onDedupUpdated(cb: (groups: DuplicateGroup[]) => void): () => void {
  return subscribe<DuplicateGroup[]>("dedup:updated", cb);
}

/**
 * Subscribes to menu:open-folder-dialog, fired by the native "ファイル >
 * フォルダ管理..." menu item. Payload-less, like onCatalogChanged -- the
 * event only signals "open the dialog".
 */
export function onMenuOpenFolderDialog(callback: () => void): () => void {
  return subscribe<void>("menu:open-folder-dialog", callback);
}

/**
 * Subscribes to menu:open-wb-import-dialog, fired by the native "ファイル >
 * .wbインポート..." menu item.
 */
export function onMenuOpenWbImportDialog(callback: () => void): () => void {
  return subscribe<void>("menu:open-wb-import-dialog", callback);
}

/**
 * Subscribes to menu:about, fired by the native "ヘルプ > バージョン情報"
 * menu item.
 */
export function onMenuAbout(callback: () => void): () => void {
  return subscribe<void>("menu:about", callback);
}

/**
 * Subscribes to menu:style-selected, fired by the native "スタイル" submenu.
 * Payload is the selected style id (currently only `"default"`).
 */
export function onMenuStyleSelected(callback: (style: string) => void): () => void {
  return subscribe<string>("menu:style-selected", callback);
}
