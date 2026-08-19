// Regression test for the "grid not displaying" bug.
//
// History of this investigation (see App.css's own comments on `.app` /
// `.video-list` (formerly `.thumbnail-grid` -- renamed in a later redesign,
// see this file's own note below) / the seven sibling-section rules for the
// final reasoning): the root cause is that the grid/list is a
// `flex: 1` child of `.app`, giving it a flex-basis of 0 -- whenever the
// sibling sections stacked around it collectively need more height than the
// window has, CSS flexbox's shrink algorithm assigns this flex-basis-0 item
// exactly 0 of the negative free space, regardless of what the siblings
// themselves do (measured and confirmed: giving the siblings `min-height: 0`
// + `max-height` + `overflow: auto`, so each is fully self-contained and
// none of them visually overlap another section, does NOT by itself give
// the grid/list back any height -- its scaled shrink factor is 0 no matter
// how compressible the siblings are). The adopted fix combines:
//   1. A bigger default/minimum window size (tauri.conf.json: 1280x900
//      default, 900x580 minimum) so the sibling sections' natural
//      (unclamped) height plus the grid/list's own floor fit without
//      needing to shrink at all in the common case.
//   2. The grid/list given an explicit `min-height` floor (one row --
//      ROW_HEIGHT in ThumbnailGrid.tsx, confirmed by measurement to equal
//      the real rendered row height exactly) as the actual guarantee,
//      independent of window size.
//   3. `.app`'s `overflow-y: auto` as a last-resort release valve for
//      whatever still doesn't fit (e.g. a lot of real content) -- scrolling
//      the whole page rather than silently clipping or squeezing a sibling
//      into an unreadable sliver.
//   4. The sibling sections' own `min-height: 0` + `max-height` +
//      `overflow: auto`, kept for its own independent value: it stops
//      different sections from visually overlapping each other when space
//      is tight, regardless of what happens to the grid/list.
//
// UI/UX re-design note: the header/status-bar/dialog structure this file
// drives changed substantially since the investigation above --
// `h1=GrayBrowser`, the always-visible "スキャン開始" button, and the
// SkippedFilesPanel/DuplicateGroupsPanel's own toggle buttons are all gone
// (see e2e/uiFlows.mjs's own comments for what replaced each). The layout
// mechanics under test (flex-basis-0 shrink, the min-height floor) are
// unchanged -- only the selectors/flow used to reach a populated grid were
// updated. See layout-shrink-regression.e2e.mjs for two additional
// regression tests (horizontal shrink, 122px non-grid budget) this file's
// sibling adds.
//
// UI/UX re-design note: a later redesign replaced the multi-column
// thumbnail grid with a one-video-per-row list (`ThumbnailGrid.tsx` still
// hosts it, but the DOM/CSS classes changed: `.thumbnail-grid` ->
// `.video-list`, `.thumbnail-cell` -> `.video-row`, ROW_HEIGHT 180 -> 258).
// This file's selectors/constant below were updated to match; the
// flex-basis-0 shrink mechanics and the tests' own structure/assertions are
// otherwise unchanged from the history above.
//
// This differs from incremental-search.e2e.mjs's existing `.video-row`
// wait in that it isolates the assertion to layout metrics (container
// height) rather than only a row-existence timeout, so a failure here
// points straight at the CSS/virtualizer layer instead of looking like a
// generic timeout that could be blamed on IPC/backend timing.
//
// All three tests below seed `watch_folders` directly into app.db *before*
// launching the WebDriver session, rather than seeding then calling
// `browser.refresh()` mid-session (the pattern incremental-search.e2e.mjs/
// tag-management.e2e.mjs use). This investigation found `browser.refresh()`
// (and even `location.reload()` executed in-page) to be unreliable against
// this local tauri-driver + WebView2 combination: the frontend's Tauri IPC
// calls issued right after a reload intermittently never resolve, leaving
// every `useEffect`-driven fetch (list_watch_folders, list_videos, ...)
// stuck at its initial empty state even though app.db on disk already has
// the seeded data (verified via a direct read-back). Seeding before the
// session even exists sidesteps that reload-time IPC flakiness entirely,
// since the app's normal cold-boot path already reads settings fresh from
// disk -- no reload involved.
import assert from "node:assert/strict";
import { test } from "node:test";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { cleanupFixtureFolder, createFixtureFolder, seedWatchFolder } from "../fixtures.mjs";
import { saveFailureScreenshot, createSession } from "../session.mjs";
import { ensureAppDbExists, scanViaFolderDialog, searchAndWaitForCellCount, waitForAppReady } from "../uiFlows.mjs";

// tauri.conf.json's confirmed values (defaults and minimum). minHeight was
// changed from 850 to 580: the native menu bar does not consume WebView2
// client-area height on this platform, so no extra margin beyond the 560px
// layout calculation was needed; confirmed empirically via the two
// window-size tests below, which stayed green against the new value.
const DEFAULT_WINDOW = { width: 1280, height: 900 };
const MIN_WINDOW = { width: 900, height: 580 };
// ROW_HEIGHT in ThumbnailGrid.tsx / `.video-list`'s `min-height` floor in
// App.css -- kept as one named constant so the two can't silently drift
// apart without this test file itself needing an update. A row hosts 6
// thumbnails + metadata/rating/tag editor, so this floor needs enough
// height to fit the full row. Measured via CDP
// (WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port) against
// real dev data at the "one tag assigned" baseline -- see
// ThumbnailGrid.tsx's own ROW_HEIGHT comment for the exact arithmetic. One
// methodology note from that measurement worth keeping here: reading
// `scrollHeight` on a `height`-overridden element with `overflow-y: auto`
// induces a vertical scrollbar that narrows the element's own content
// width, which skews tag-wrapping measurements -- overriding `align-self`
// instead avoids that bias.
const GRID_MIN_HEIGHT_PX = 226;

// The three tests above only pin down the CSS-side floor (`.video-list`'s
// `min-height: 226px`) -- none of them fail if `tauri.conf.json`'s own
// default/minimum window size (part ① and ② of the actual fix; see this
// file's header) is reverted, because they all call `browser.setWindowSize()`
// themselves before asserting anything, which overrides whatever the app
// launched at. The two tests below close that gap.
const TAURI_CONF_PATH = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
  "..",
  "src-tauri",
  "tauri.conf.json",
);

async function readLayout(browser) {
  return browser.execute(() => {
    const app = document.querySelector(".app");
    // `.video-list` (formerly `.thumbnail-grid`, renamed in a later
    // redesign) -- kept the `grid`/`cellCount` field names below unchanged
    // (only the selectors they read change) since every assertion in this
    // file already refers to them and a pure rename would be an unrelated
    // diff.
    const grid = document.querySelector(".video-list");
    return {
      windowInnerWidth: window.innerWidth,
      windowInnerHeight: window.innerHeight,
      appOffsetHeight: app ? app.offsetHeight : null,
      appClientHeight: app ? app.clientHeight : null,
      appScrollHeight: app ? app.scrollHeight : null,
      cellCount: document.querySelectorAll(".video-row").length,
      gridExists: !!grid,
      gridOffsetHeight: grid ? grid.offsetHeight : null,
      gridClientHeight: grid ? grid.clientHeight : null,
      gridScrollHeight: grid ? grid.scrollHeight : null,
    };
  });
}

test("default window size (1280x900) shows a full row of thumbnails", async () => {
  await ensureAppDbExists();

  const token = `e2e${Date.now()}default`;
  // Enough fixtures to guarantee at least one full row regardless of
  // column count at this window width (CELL_WIDTH = 200 in
  // ThumbnailGrid.tsx; 1280px wide comfortably fits more than 3 columns).
  const fixtureDir = createFixtureFolder([
    `${token}_a.mp4`,
    `${token}_b.mp4`,
    `${token}_c.mp4`,
  ]);
  seedWatchFolder(fixtureDir);

  const browser = await createSession();
  try {
    await browser.setWindowSize(DEFAULT_WINDOW.width, DEFAULT_WINDOW.height);
    await waitForAppReady(browser);
    await scanViaFolderDialog(browser);

    await searchAndWaitForCellCount(browser, token, 3);
    await browser.pause(500);

    const layout = await readLayout(browser);
    assert.ok(layout.gridExists, `expected .video-list to exist, got ${JSON.stringify(layout)}`);
    assert.ok(
      layout.gridClientHeight >= GRID_MIN_HEIGHT_PX,
      `expected .video-list clientHeight >= ${GRID_MIN_HEIGHT_PX} (a full row) at the default window size, got ${JSON.stringify(layout)}`,
    );
    assert.equal(
      layout.cellCount,
      3,
      `expected exactly 3 .video-row (this run's own fixtures), got ${JSON.stringify(layout)}`,
    );
  } catch (e) {
    await saveFailureScreenshot(browser, "grid-visible-default-size");
    throw e;
  } finally {
    await browser.deleteSession().catch(() => {});
    cleanupFixtureFolder(fixtureDir);
  }
});

test("minimum window size (900x580) keeps the grid at its floor without triggering page scroll", async () => {
  await ensureAppDbExists();

  const token = `e2e${Date.now()}minsize`;
  const fixtureDir = createFixtureFolder([`${token}_a.mp4`]);
  seedWatchFolder(fixtureDir);

  const browser = await createSession();
  try {
    await browser.setWindowSize(MIN_WINDOW.width, MIN_WINDOW.height);
    await waitForAppReady(browser);
    await scanViaFolderDialog(browser);

    await searchAndWaitForCellCount(browser, token, 1);
    await browser.pause(500);

    const layout = await readLayout(browser);
    assert.ok(layout.gridExists, `expected .video-list to exist, got ${JSON.stringify(layout)}`);
    assert.ok(
      layout.gridClientHeight >= GRID_MIN_HEIGHT_PX,
      `expected .video-list clientHeight >= ${GRID_MIN_HEIGHT_PX} at the minimum window size, got ${JSON.stringify(layout)}`,
    );
    assert.equal(
      layout.cellCount,
      1,
      `expected exactly 1 .video-row (this run's own fixture), got ${JSON.stringify(layout)}`,
    );
    // At the minimum window size, with only this test's own minimal
    // content and the status panel closed, `.app`'s natural content is
    // sized (tauri.conf.json's minWidth/minHeight were chosen precisely so
    // this fits -- see this file's header comment) to fit without needing
    // its `overflow-y: auto` release valve at all.
    assert.ok(
      layout.appScrollHeight <= layout.appClientHeight,
      `expected .app to NOT need to scroll at the minimum window size, got ${JSON.stringify(layout)}`,
    );
  } catch (e) {
    await saveFailureScreenshot(browser, "grid-visible-minimum-size");
    throw e;
  } finally {
    await browser.deleteSession().catch(() => {});
    cleanupFixtureFolder(fixtureDir);
  }
});

test("thumbnail grid stays at its floor even with the status panel open and full of real content", async () => {
  await ensureAppDbExists();

  // UI/UX re-design note: previously, SkippedFilesPanel/
  // DuplicateGroupsPanel/GenerationFailuresPanel were independently
  // toggleable flex children of `.app`, so this test used to open all three
  // at once to stress the sibling sections' combined height. Now, StatusBar's
  // three badges share a single popover slot (`.status-panel`,
  // `position: absolute`, anchored to the status bar, 26px tall) --
  // only one can be open at a time, and (unlike the old flex-child panels)
  // an open popover can never compete with `.video-list` for flex
  // shrink-share in the first place, since it isn't a flex child of `.app`
  // at all. This test now instead verifies: (a) a real scan really does
  // populate both the skipped-files and duplicate-groups badges with
  // nonzero counts, and (b) opening the richer of the two panels (duplicate
  // groups: renders member thumbnails, file names, paths) still leaves
  // `.video-list` at its floor height, exactly like the sparser
  // "nothing open" case above.
  const token = `e2e${Date.now()}siblingsopen`;
  // Two byte-identical fixtures register as a quick_hash duplicate pair --
  // `start_scan` fires `dedup::refresh_duplicate_groups` in the background
  // after every scan, so DuplicateGroupsPanel ends up with real, nonzero
  // content. One filename with a machine-dependent character populates
  // SkippedFilesPanel the same way.
  const dupContent = "identical duplicate content for the grid-visible-after-scan e2e regression test";
  const fixtureDir = createFixtureFolder([`${token}_grid_visible.mp4`, `${token}①_skip.mp4`]);
  fs.writeFileSync(path.join(fixtureDir, `${token}_dup_a.mp4`), dupContent);
  fs.writeFileSync(path.join(fixtureDir, `${token}_dup_b.mp4`), dupContent);
  seedWatchFolder(fixtureDir);

  const browser = await createSession();
  try {
    await browser.setWindowSize(DEFAULT_WINDOW.width, DEFAULT_WINDOW.height);
    await waitForAppReady(browser);
    await scanViaFolderDialog(browser);

    // Dedup/skip detection runs asynchronously in the background after the
    // scan response returns, so poll both badges' labels until they reflect
    // real, nonzero content instead of racing them.
    await browser.waitUntil(
      async () => {
        const info = await browser.execute(() => ({
          skip: document.querySelector('[data-testid="status-badge-unregistered"]')?.textContent ?? null,
          dup: document.querySelector('[data-testid="status-badge-duplicates"]')?.textContent ?? null,
        }));
        return (
          info.skip !== null &&
          /未登録\s*[1-9]/.test(info.skip) &&
          info.dup !== null &&
          /重複\s*[1-9]/.test(info.dup)
        );
      },
      {
        timeout: 30_000,
        interval: 1000,
        timeoutMsg: "status-badge-unregistered/status-badge-duplicates never reported nonzero content",
      },
    );

    // Open the duplicate-groups popover -- the richer of the two (member
    // thumbnails + file names + paths), which is what "full of real
    // content" stresses here.
    const duplicatesBadge = await browser.$('[data-testid="status-badge-duplicates"]');
    await duplicatesBadge.click();
    await browser.$('[data-testid="status-panel"]').waitForExist({ timeout: 10_000 });
    await browser.$(".duplicate-group").waitForExist({ timeout: 10_000 });
    await browser.pause(500);

    // Narrow to just the one grid_visible fixture -- `token` alone would
    // also match this run's two duplicate fixtures (`${token}_dup_a.mp4`/
    // `${token}_dup_b.mp4`), which are deliberately registered too (to
    // populate DuplicateGroupsPanel) but aren't the video under test here.
    await searchAndWaitForCellCount(browser, `${token}_grid_visible`, 1);
    await browser.pause(500);

    const layout = await readLayout(browser);
    assert.ok(layout.gridExists, `expected .video-list to exist, got ${JSON.stringify(layout)}`);
    assert.ok(
      layout.gridClientHeight >= GRID_MIN_HEIGHT_PX,
      `expected .video-list clientHeight to never drop below its ${GRID_MIN_HEIGHT_PX}px floor even with the duplicate-groups panel open, got ${JSON.stringify(layout)}`,
    );
    assert.equal(
      layout.cellCount,
      1,
      `expected exactly 1 .video-row (this run's own grid-visible fixture), got ${JSON.stringify(layout)}`,
    );
  } catch (e) {
    await saveFailureScreenshot(browser, "grid-visible-siblings-open");
    throw e;
  } finally {
    await browser.deleteSession().catch(() => {});
    cleanupFixtureFolder(fixtureDir);
  }
});

test("app launches at tauri.conf.json's configured default size (1280x900), before any test-driven resize", async () => {
  await ensureAppDbExists();

  const browser = await createSession();
  try {
    // Deliberately the *only* test in this file that never calls
    // `browser.setWindowSize()` -- every test above sets an explicit size
    // before asserting anything, which is exactly why none of them would
    // catch tauri.conf.json's `app.windows[0].width`/`height` (part ① of
    // the fix) being reverted. This observes the window exactly as the app
    // itself launches it.
    await waitForAppReady(browser);
    await browser.pause(300);

    const size = await browser.execute(() => ({
      innerWidth: window.innerWidth,
      innerHeight: window.innerHeight,
    }));
    assert.ok(
      size.innerWidth >= MIN_WINDOW.width && size.innerHeight >= MIN_WINDOW.height,
      `expected the app's default launch size (window.innerWidth/innerHeight, no setWindowSize call) ` +
        `to be at least ${MIN_WINDOW.width}x${MIN_WINDOW.height}, got ${JSON.stringify(size)}`,
    );
  } catch (e) {
    await saveFailureScreenshot(browser, "grid-visible-default-launch-size");
    throw e;
  } finally {
    await browser.deleteSession().catch(() => {});
  }
});

// This test intentionally does NOT drive a real WebDriver session the way
// a first approach attempted (`browser.setWindowSize(800, 600)` --
// tauri.conf.json's old default/no-minimum values -- then reading back
// window.innerWidth/innerHeight to confirm it got clamped to
// minWidth:900/minHeight:580). That approach was tried first and found to
// not work at all, not even after polling for several seconds:
// WebView2Driver's `Set Window Rect` command (what `setWindowSize()` sends)
// resizes the OS window via a direct `SetWindowPos` call, which does not
// go through the `WM_GETMINMAXINFO` negotiation tao/winit relies on to
// enforce `minWidth`/`minHeight` -- that negotiation is only triggered by
// *interactive* resizing (dragging the window border, aero-snap, etc.), not
// by a program setting the window's rect directly. Confirmed empirically
// against this fix's own build: after `setWindowSize(800, 600)`, the window
// stayed at exactly 800x600 for 4+ seconds of polling, never drifting back
// toward its configured minimum. So `minWidth`/`minHeight` genuinely
// protects only a user manually dragging the window smaller -- a gesture
// WebdriverIO has no direct, non-brittle way to simulate (it would require
// raw OS-level mouse drag events at the exact screen coordinates of the
// resize border, which is far more fragile than what it would be
// protecting). Given that, this test instead asserts directly against
// tauri.conf.json itself -- the actual single source of truth for both ①
// and ②'s values -- so a future edit to either still turns this red, just
// without going through a WebDriver session at all (this test needs no
// browser/app launch). This same WebView2Driver quirk has a flip side:
// it's exactly what makes the horizontal-shrink regression test in
// layout-shrink-regression.e2e.mjs *possible* to write as a real,
// non-brittle WebDriver assertion.
test("tauri.conf.json still declares the default/minimum window size this suite depends on", () => {
  const conf = JSON.parse(fs.readFileSync(TAURI_CONF_PATH, "utf8"));
  const win = conf.app?.windows?.[0] ?? {};
  assert.equal(
    win.width,
    DEFAULT_WINDOW.width,
    `expected tauri.conf.json's app.windows[0].width to be ${DEFAULT_WINDOW.width}, got ${JSON.stringify(win)}`,
  );
  assert.equal(
    win.height,
    DEFAULT_WINDOW.height,
    `expected tauri.conf.json's app.windows[0].height to be ${DEFAULT_WINDOW.height}, got ${JSON.stringify(win)}`,
  );
  assert.equal(
    win.minWidth,
    MIN_WINDOW.width,
    `expected tauri.conf.json's app.windows[0].minWidth to be ${MIN_WINDOW.width}, got ${JSON.stringify(win)}`,
  );
  assert.equal(
    win.minHeight,
    MIN_WINDOW.height,
    `expected tauri.conf.json's app.windows[0].minHeight to be ${MIN_WINDOW.height}, got ${JSON.stringify(win)}`,
  );
});
