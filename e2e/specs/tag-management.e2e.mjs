// E2E flow: tag assignment/removal.
import assert from "node:assert/strict";
import { test } from "node:test";
import { cleanupFixtureFolder, createFixtureFolder, seedWatchFolder } from "../fixtures.mjs";
import { createSession, saveFailureScreenshot } from "../session.mjs";
import { scanViaFolderDialog, waitForAppReady } from "../uiFlows.mjs";

test("tag can be assigned to a video and removed again", async () => {
  // Run-unique so this test interacts with its own freshly-scanned video,
  // not a leftover row from a previous run -- app.db is a real, persistent
  // file across runs (see incremental-search.e2e.mjs for the same reasoning).
  const token = `e2e${Date.now()}`;
  const fixtureDir = createFixtureFolder([`${token}_tag_test.mp4`]);
  const browser = await createSession();

  try {
    // First launch just needs to reach "app is up" so db::init has created
    // app.db/the settings table -- the actual folder registration happens
    // via direct DB write (fixtures.seedWatchFolder), not the native picker.
    await waitForAppReady(browser);

    seedWatchFolder(fixtureDir);
    // Force a full remount so FolderSidebar's mount-time listWatchFolders()
    // call picks up the row just written directly to the DB. (Known flaky
    // against this local tauri-driver + WebView2 combination -- see
    // grid-visible-after-scan.e2e.mjs's header comment for the
    // investigation -- kept as-is here, not this file's scope to fix.)
    await browser.refresh();
    await waitForAppReady(browser);

    await scanViaFolderDialog(browser);

    // Scope to this run's fixture via the search box so the tag form below
    // unambiguously targets the video this test just registered, not a
    // stale one from a previous run.
    const searchBox = await browser.$('[data-testid="header-search-input"]');
    await searchBox.waitForExist({ timeout: 30_000 });
    await searchBox.click();
    await browser.keys(token.split(""));

    // UI/UX re-design note: the detail panel is now folded into each
    // `.video-row` -- there's no "詳細" button/separate panel to open
    // anymore (App.css's `.main-area` comment, VideoRow.tsx). The tag
    // editor is always rendered inline inside the row, so once the search
    // above has narrowed the list to this run's single fixture, waiting for
    // that one `.video-row` to exist is enough before interacting with its
    // (already-mounted) tag form directly.
    const videoRow = await browser.$('[data-testid="video-row"]');
    await videoRow.waitForExist({ timeout: 30_000 });

    const tagInput = await browser.$('input[placeholder="タグを追加"]');
    await tagInput.waitForExist({ timeout: 10_000 });
    await tagInput.setValue("e2e-smoke-tag");
    await browser.$('button=追加').click();

    const chip = await browser.$("span.tag-chip=e2e-smoke-tag");
    await chip.waitForExist({ timeout: 10_000 });
    assert.ok(await chip.isExisting(), "tag chip should appear after assigning");

    // The chip's own "×" remove button -- scoped inside the chip so this
    // doesn't accidentally match another chip's remove button.
    const removeButton = await chip.$("button");
    await removeButton.click();
    await chip.waitForExist({ timeout: 10_000, reverse: true });
    assert.ok(!(await chip.isExisting()), "tag chip should disappear after removing");
  } catch (e) {
    await saveFailureScreenshot(browser, "tag-management");
    throw e;
  } finally {
    await browser.deleteSession().catch(() => {});
    cleanupFixtureFolder(fixtureDir);
  }
});
