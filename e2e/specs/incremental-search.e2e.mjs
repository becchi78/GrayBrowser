// E2E flow: incremental search filtering.
import assert from "node:assert/strict";
import { test } from "node:test";
import { cleanupFixtureFolder, createFixtureFolder, seedWatchFolder } from "../fixtures.mjs";
import { createSession, saveFailureScreenshot } from "../session.mjs";
import { scanViaFolderDialog, waitForAppReady } from "../uiFlows.mjs";

test("typing in the search box filters the list to matching videos only", async () => {
  // A run-unique token in every fixture file name scopes every assertion
  // below to just this run's rows via the search box itself, rather than
  // asserting on the whole list's raw row count -- app.db is a real,
  // shared, persistent file across runs (there's no DB reset between test
  // invocations), so past runs' registered videos are still in there.
  const token = `e2e${Date.now()}`;
  const fixtureDir = createFixtureFolder([`${token}_alpha_movie.mp4`, `${token}_beta_show.mp4`]);
  const browser = await createSession();

  // Types `term` into the search box (replacing any existing content), then
  // polls the list's row count until it stops changing for two consecutive
  // checks -- covers the 200ms debounce plus IPC/query round trip without
  // depending on their exact timing.
  //
  // UI/UX re-design note: counts `.video-row` (one row per video) rather
  // than the old `.thumbnail-cell`. Kept the `searchAndCountCells` name for
  // the same reason as uiFlows.mjs's `searchAndWaitForCellCount` -- purely
  // local to this file, but consistent with that decision.
  async function searchAndCountCells(term) {
    const searchBox = await browser.$('[data-testid="header-search-input"]');
    await searchBox.click();
    await browser.keys(["Control", "a"]);
    await browser.keys(term ? term.split("") : ["Backspace"]);

    let lastCount = -1;
    await browser.waitUntil(
      async () => {
        const count = (await browser.$$(".video-row")).length;
        const stable = count === lastCount;
        lastCount = count;
        return stable;
      },
      {
        timeout: 10_000,
        interval: 300,
        timeoutMsg: `timed out waiting for the row count to stabilize for search "${term}"`,
      },
    );
    return lastCount;
  }

  try {
    await waitForAppReady(browser);

    seedWatchFolder(fixtureDir);
    // Force a full remount so FolderSidebar's mount-time listWatchFolders()
    // call picks up the row just written directly to the DB.
    await browser.refresh();
    await waitForAppReady(browser);

    await scanViaFolderDialog(browser);
    // Give the scan a moment to register files before searching starts.
    await browser.$(".video-row").waitForExist({ timeout: 30_000 });

    // Scope to this run's two fixtures via the token before asserting
    // anything -- proves both registered, without depending on the rest of
    // app.db being empty.
    assert.equal(
      await searchAndCountCells(token),
      2,
      "expected both fixture videos to appear when searching by this run's token",
    );

    // Narrowing to "token alpha" (multi-term AND) should leave exactly one.
    assert.equal(
      await searchAndCountCells(`${token} alpha`),
      1,
      "expected the list to filter down to the matching video",
    );
    // UI/UX re-design note: the old check here read `.thumbnail-name`'s
    // text to confirm the *surviving* row was actually the alpha fixture,
    // not just that the count was 1. That element no longer exists --
    // VideoRow renders no visible file-name text at all; the only
    // remaining DOM trace of the file name is the `alt` attribute on the
    // row's thumbnail `<img>`
    // (VideoRow.tsx), but this suite's fixture files are deliberately fake,
    // non-decodable bytes (fixtures.mjs's own comment) -- ffmpeg/ffprobe can
    // never successfully generate a thumbnail against them, so that `<img>`
    // never actually renders (VideoRow.tsx: only a `.thumbnail-placeholder`
    // div shows while `thumbnails` is null, which is permanently the case
    // here) and can't be used as a selector. Instead, cross-checking that
    // narrowing to the *other* fixture's own distinguishing term ("beta")
    // also independently narrows to exactly one gives the same "search
    // actually discriminates between the two fixtures, not just an
    // incidental count of 1" guarantee that the old text check gave, without
    // depending on DOM content this component no longer renders.
    assert.equal(
      await searchAndCountCells(`${token} beta`),
      1,
      "expected the list to filter down to the other fixture when searching its own distinguishing term",
    );

    // Widening back to just the token restores both.
    assert.equal(
      await searchAndCountCells(token),
      2,
      "expected widening the search back to the token to restore both videos",
    );
  } catch (e) {
    await saveFailureScreenshot(browser, "incremental-search");
    throw e;
  } finally {
    await browser.deleteSession().catch(() => {});
    cleanupFixtureFolder(fixtureDir);
  }
});
