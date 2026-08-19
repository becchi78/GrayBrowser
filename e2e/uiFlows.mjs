// Shared WebDriver interaction helpers for the header/dialog UI.
// Centralizing these keeps the several spec files that all need "wait for
// the app shell" / "seed a folder then run a scan" / "search and wait for
// an exact cell count" in sync with the current DOM structure, rather than
// duplicating the same selectors across files -- duplication is exactly how
// the previous `h1=GrayBrowser` / `button=スキャン開始` selectors went stale
// in every spec file at once.

import { DatabaseSync } from "node:sqlite";
import { appDbPath, createSession } from "./session.mjs";

// `<h1>` was removed from the header -- the header's first row
// (`data-testid="header-row-primary"`) is always mounted (unconditionally,
// unlike the detail panel/dialogs) and is the closest equivalent
// "the app shell is up" signal left in the DOM.
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

// Runs a scan via FolderDialog: opens the dialog through the sidebar's
// "フォルダ管理 ▸" link, triggers the dialog's own scan button, waits for the
// scan summary, then closes the dialog again. This replaces the previous
// always-visible "スキャン開始" button, which moved into FolderDialog (see
// that component's own handleScan comment for why a scan trigger lives
// there despite not being among the dialog's listed footer buttons).
// Assumes the caller has already seeded the folder to scan
// via fixtures.mjs's `seedWatchFolder()` -- WebDriver cannot drive the
// native folder picker `pickWatchFolders()` would otherwise open, so this
// never exercises "+ フォルダを追加" itself.
export async function scanViaFolderDialog(browser, { scanTimeout = 30_000 } = {}) {
  const manageLink = await browser.$('[data-testid="folder-sidebar-manage-link"]');
  await manageLink.waitForExist({ timeout: 10_000 });
  await manageLink.click();

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
// Kept the `searchAndWaitForCellCount` name (still counts `.video-row`
// elements one-per-video, exactly what "cell" meant before the list-view
// redesign) rather than renaming to a "row" equivalent, since every caller
// across the spec files already imports it by this name -- renaming here
// would just ripple an unrelated import-only diff into every one of them
// for no behavioral change.
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
