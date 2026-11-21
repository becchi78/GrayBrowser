// Shared WebDriver interaction helpers for the header/dialog UI.
// Centralizing these keeps the several spec files that all need "wait for
// the app shell" / "seed a folder then run a scan" / "search and wait for
// an exact cell count" in sync with the current DOM structure, rather than
// duplicating the same selectors across files.

import { DatabaseSync } from "node:sqlite";
import { appDbPath, createSession } from "./session.mjs";

// The header's first row (`data-testid="header-row-primary"`) is always
// mounted (unconditionally, unlike the detail panel/dialogs), so it works as
// the "the app shell is up" signal.
export async function waitForAppReady(browser, timeout = 30_000) {
  await browser.$('[data-testid="header-row-primary"]').waitForExist({ timeout });
}

// `settings` (and every other table) only exists once the app has been
// launched at least once (db::init has run). On a machine/CI runner where
// app.db already exists from a previous run (the common case), this is a
// cheap no-op check. On a truly fresh environment (first-ever run), it
// launches once just to let db::init create the schema, then closes
// immediately.
export async function ensureAppDbExists() {
  try {
    const db = new DatabaseSync(appDbPath());
    db.prepare("SELECT 1 FROM settings LIMIT 1").get();
    db.close();
    return;
  } catch {
    // fall through to bootstrap
  }
  const browser = await createSession();
  try {
    await waitForAppReady(browser);
  } finally {
    await browser.deleteSession().catch(() => {});
  }
}

// Opens the hamburger menu, opens one of its groups, and presses a leaf item.
// `groupId` is the suffix of the group's `app-menu-group-<id>` test id (file,
// tag, style, settings, help); `itemId` is the suffix of the leaf's
// `app-menu-item-<id>` (e.g. "folder", "tagBar", "settings"). The menu is
// ordinary DOM, so this is a plain click sequence; no backend event is involved.
export async function clickAppMenuItem(browser, groupId, itemId) {
  const menuButton = await browser.$('[data-testid="app-menu-button"]');
  await menuButton.waitForClickable({ timeout: 10_000 });
  await menuButton.click();
  const groupItem = await browser.$(`[data-testid="app-menu-group-${groupId}"]`);
  await groupItem.waitForClickable({ timeout: 5_000 });
  await groupItem.click();
  const item = await browser.$(`[data-testid="app-menu-item-${itemId}"]`);
  await item.waitForClickable({ timeout: 5_000 });
  await item.click();
}

// Runs a scan via FolderDialog: opens the dialog from the hamburger menu's
// "ファイル > フォルダ管理..." item, triggers the dialog's own scan button, waits for the
// scan summary, then closes the dialog again. This replaces the previous
// always-visible "スキャン開始" button, which moved into FolderDialog (see
// that component's own handleScan comment for why a scan trigger lives
// there despite not being among the dialog's listed footer buttons).
// Assumes the caller has already seeded the folder to scan
// via fixtures.mjs's `seedWatchFolder()` -- WebDriver cannot drive the
// native folder picker `pickWatchFolders()` would otherwise open, so this
// never exercises "+ フォルダを追加" itself.
export async function scanViaFolderDialog(browser, { scanTimeout = 30_000 } = {}) {
  await clickAppMenuItem(browser, "file", "folder");

  const dialog = await browser.$('[data-testid="folder-dialog"]');
  await dialog.waitForExist({ timeout: 10_000 });

  const scanButton = await browser.$('[data-testid="folder-dialog-scan-btn"]');
  await scanButton.waitForEnabled({ timeout: 25_000 });
  await scanButton.click();
  await browser.$(".scan-summary").waitForExist({ timeout: scanTimeout });

  const closeButton = await browser.$('[data-testid="folder-dialog-close-btn"]');
  await closeButton.click();
  await dialog.waitForExist({ timeout: 5_000, reverse: true });
}

// Scopes the list to just one test run's own fixtures via the search box
// (app.db is a real, persistent file shared across every run in this repo's
// e2e history), so row count assertions don't depend on whatever unrelated
// videos have accumulated in app.db over time. Polls until the count reaches
// exactly `expectedCount`, rather than just waiting a fixed duration, to
// cover the search box's debounce plus the IPC/query round trip.
//
// Counts `.video-row` elements (one per video). The "cell" in the name is a
// historical term for one video entry; it is kept because every spec file
// imports it by this name.
export async function searchAndWaitForCellCount(browser, term, expectedCount) {
  const searchBox = await browser.$('[data-testid="header-search-input"]');
  await searchBox.waitForExist({ timeout: 30_000 });
  await searchBox.click();
  await browser.keys(["Control", "a"]);
  await browser.keys(term.split(""));
  await browser.waitUntil(
    async () => (await browser.$$(".video-row")).length === expectedCount,
    {
      timeout: 15_000,
      interval: 300,
      timeoutMsg: `row count for search "${term}" never reached ${expectedCount}`,
    },
  );
}
