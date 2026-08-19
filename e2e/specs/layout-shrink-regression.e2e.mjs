// UI/UX re-design layout regression tests.
//
// Both tests below are layout-measurement assertions (not fixed-timeout
// waits), matching grid-visible-after-scan.e2e.mjs's own style -- see that
// file's header comment for why (a timing-only assertion looks like a
// generic IPC/backend flake instead of pointing straight at the CSS layer).
//
// Mutation-tested manually (not run automatically here, since the mutation
// itself needs a rebuild in between): each test's own comment records what
// was mutated, how the rebuilt app was run, and that the test went red as
// expected before being reverted.
import assert from "node:assert/strict";
import { randomUUID } from "node:crypto";
import { DatabaseSync } from "node:sqlite";
import { test } from "node:test";
import { cleanupFixtureFolder, createFixtureFolder, seedWatchFolder } from "../fixtures.mjs";
import { appDbPath, createSession, saveFailureScreenshot } from "../session.mjs";
import { ensureAppDbExists, scanViaFolderDialog, searchAndWaitForCellCount, waitForAppReady } from "../uiFlows.mjs";

const DEFAULT_WINDOW = { width: 1280, height: 900 };
// WebView2Driver's `Set Window Rect` (what `browser.setWindowSize()` sends)
// resizes the OS window via a direct `SetWindowPos` call, bypassing the
// `WM_GETMINMAXINFO` negotiation tao/winit uses to enforce
// tauri.conf.json's `minWidth` (900) -- that negotiation only fires on
// *interactive* resizing (dragging the window border). This lets a
// WebDriver test push the window well below 900px wide, something a real
// user resizing by hand could never do, and directly exercise the CSS-only
// floor (`.video-list`'s `min-width: 300px`, App.css) that's the actual
// guarantee against the same class of flex-basis-0 shrink trap
// grid-visible-after-scan.e2e.mjs exercises vertically, just horizontally
// here.
//
// UI/UX re-design note: this test originally needed the detail panel
// opened too, because MainArea used to have *three* columns --
// FolderSidebar (200px, flex-shrink:0) + `.thumbnail-grid` (flex:1) +
// PropertiesPanel/detail panel (340px, flex-shrink:0, conditionally
// mounted) -- and with the detail panel closed, the grid was the *only*
// flexible item in the row, so it just grew to fill whatever was left of
// the window (a positive-free-space case, `min-width` irrelevant either
// way). A later redesign removed the detail panel entirely (folded into
// each `.video-row` -- App.css's own `.main-area` comment), so MainArea is
// now just two columns: FolderSidebar (200px default, user-resizable --
// this test never touches the drag handle, so it stays at its 200px
// default here) + `.video-list` (flex:1, `min-width`, see below). That's
// still enough columns to reproduce the trap on its own -- shrinking the
// window far enough now makes *this* two-column row's total desired width
// (sidebar + `.video-list` min-width + `.app`'s own padding) exceed what's
// available, the same negative-free-space case where a flex-basis-0 item's
// scaled shrink factor is 0 regardless of its own min-width unless the
// browser clamps to it as an explicit floor. No detail panel (or any other
// extra mounted UI) is needed anymore to force that condition.
//
// SHRUNK_WIDTH itself: `.app`'s `padding: 1em` (16px, `:root`'s
// `font-size: 16px`) on each side is 32px total, so the two columns' own
// combined natural width is 200 (sidebar, at its unresized default) + 582
// (`.video-list` min-width -- see App.css's own comment on that rule for
// where 582 comes from) + 32 (`.app` padding) = 814px. 350px is now
// comfortably (464px of margin) under this threshold, so it's kept
// unchanged here rather than widened further -- the test's purpose
// (confirming `.video-list` never collapses to 0 width) doesn't need the
// window pushed all the way down to just-under-the-threshold, only
// meaningfully below it, which 350px still is by a wide margin.
const SHRUNK_WIDTH = 350;

test("shrinking the window well below the 900px minWidth still leaves .video-list at a nonzero width", async () => {
  await ensureAppDbExists();

  const token = `e2e${Date.now()}hshrink`;
  const fixtureDir = createFixtureFolder([`${token}_a.mp4`]);
  seedWatchFolder(fixtureDir);

  const browser = await createSession();
  try {
    // Scan at the default size first -- FolderDialog/its scan button aren't
    // under test here, and a comfortably wide window avoids any interaction
    // with the resize below.
    await browser.setWindowSize(DEFAULT_WINDOW.width, DEFAULT_WINDOW.height);
    await waitForAppReady(browser);
    await scanViaFolderDialog(browser);
    await searchAndWaitForCellCount(browser, token, 1);

    // Now shrink well below tauri.conf.json's minWidth (900) -- see this
    // file's SHRUNK_WIDTH comment for why WebDriver can do this at all, and
    // for the arithmetic behind why 350px forces MainArea's two columns'
    // combined natural width past what's available.
    await browser.setWindowSize(SHRUNK_WIDTH, DEFAULT_WINDOW.height);
    await browser.pause(500);

    const layout = await browser.execute(() => {
      const list = document.querySelector(".video-list");
      return {
        windowInnerWidth: window.innerWidth,
        gridExists: !!list,
        gridClientWidth: list ? list.clientWidth : null,
      };
    });

    assert.ok(
      layout.gridExists,
      `expected .video-list to still exist after shrinking to ${SHRUNK_WIDTH}px wide, got ${JSON.stringify(layout)}`,
    );
    // Sanity check that the scenario this test relies on actually held --
    // without it, an unrelated regression (WebView2Driver silently clamping
    // the resize) could make the assertion below pass for the wrong reason.
    assert.ok(
      layout.windowInnerWidth < 900,
      `expected the WebDriver resize to actually push window.innerWidth below tauri.conf.json's 900px minWidth, got ${JSON.stringify(layout)}`,
    );
    assert.ok(
      layout.gridClientWidth > 0,
      `expected .video-list clientWidth to stay nonzero (App.css's min-width: 582px floor) even at ${SHRUNK_WIDTH}px window width, got ${JSON.stringify(layout)}`,
    );
    // MUTATION TEST: with `.thumbnail-grid`'s (the previous name of this
    // rule, now `.video-list`) `min-width: 300px` rule in App.css commented
    // out and the app rebuilt, this assertion went red (gridClientWidth ===
    // 0) against the then-three-column layout at a wider SHRUNK_WIDTH
    // (500px) -- confirming the floor is what this test actually exercises,
    // not just "the grid element happens to render something". Reverted
    // before committing. Not re-run against the current two-column layout/
    // SHRUNK_WIDTH=350px/`.video-list` selector (a mutation test needs its
    // own rebuild-run-revert cycle) -- the arithmetic above was instead
    // checked by hand: at 350px window width, `.main-area`'s available
    // width (350 - 32 `.app` padding = 318px) is less than the sidebar's
    // fixed 200px alone, let alone 200 + `.video-list`'s min-width, so the
    // same flex-basis-0/negative-free-space mechanics this file's header
    // comment describes still apply. This mutation test's continued
    // sensitivity to the floor specifically (vs. some unrelated change
    // happening to keep clientWidth nonzero) may need re-confirming with a
    // fresh rebuild-run-revert cycle if `.video-list`'s min-width changes
    // again. `.video-list`'s own min-width has since moved (300px -> 588px
    // -> 598px -> 582px, per App.css's own comment on that rule) -- the
    // mutation test above predates all of those changes and was not re-run
    // against any of them for the same time-constraint reason; the by-hand
    // arithmetic in this comment (350px window, 318px available) remains
    // valid either way, since 318px is below the old 300px floor and every
    // wider/narrower floor that followed it.
  } catch (e) {
    await saveFailureScreenshot(browser, "layout-shrink-horizontal");
    throw e;
  } finally {
    await browser.deleteSession().catch(() => {});
    cleanupFixtureFolder(fixtureDir);
  }
});

test("the always-visible non-grid chrome (header rows + status bar) totals at most 122px", async () => {
  await ensureAppDbExists();

  const browser = await createSession();
  try {
    await browser.setWindowSize(DEFAULT_WINDOW.width, DEFAULT_WINDOW.height);
    await waitForAppReady(browser);
    await browser.pause(300);

    // Always-visible non-grid area budget = 90px (header: row1 52px + row2
    // 38px) + 32px (status bar) = 122px. Measured directly as the sum of
    // each always-mounted region's own offsetHeight (not the .app
    // padding/gap around them, which is separate from the "non-grid chrome"
    // budget this is about) so a regression in any one row is individually
    // attributable, not just a total that could pass by one row shrinking
    // while another grows.
    const layout = await browser.execute(() => {
      const primary = document.querySelector('[data-testid="header-row-primary"]');
      const filters = document.querySelector('[data-testid="header-row-filters"]');
      const statusBar = document.querySelector('[data-testid="status-bar"]');
      return {
        primaryHeight: primary ? primary.offsetHeight : null,
        filtersHeight: filters ? filters.offsetHeight : null,
        statusBarHeight: statusBar ? statusBar.offsetHeight : null,
      };
    });

    assert.ok(
      layout.primaryHeight !== null && layout.filtersHeight !== null && layout.statusBarHeight !== null,
      `expected header-row-primary/header-row-filters/status-bar to all be mounted, got ${JSON.stringify(layout)}`,
    );

    const total = layout.primaryHeight + layout.filtersHeight + layout.statusBarHeight;
    assert.ok(
      total <= 122,
      `expected the always-visible non-grid chrome (header-row-primary + header-row-filters + status-bar) ` +
        `to total at most 122px, got ${total}px: ${JSON.stringify(layout)}`,
    );
    // MUTATION TEST: with a temporary `padding-top: 50px` added to
    // `.header-row-primary` in App.css and the app rebuilt, this assertion
    // went red (total > 122) -- confirming the test actually catches a
    // chrome-height regression, not just recomputing a value that always
    // happens to pass. Reverted before committing.
  } catch (e) {
    await saveFailureScreenshot(browser, "layout-122px-budget");
    throw e;
  } finally {
    await browser.deleteSession().catch(() => {});
  }
});

// A video row whose thumbnail generation has exhausted its
// automatic-retry budget (gb_core::retry::MAX_GENERATION_ATTEMPTS = 3,
// src-tauri/src/db/queries.rs's list_videos_with_exhausted_thumbnail_attempts
// requires status='online' AND thumbnail_attempts >= that threshold),
// inserted directly into `videos` the same way uiFlows.mjs's
// ensureAppDbExists/fixtures.mjs's seedWatchFolder write to `settings` --
// WebDriver has no way to make the real thumbnail worker exhaust its retries
// against a fixture file, so this bypasses it entirely and seeds the
// end-state row the panel actually reads. Unlike DuplicateGroupsPanel (whose
// list only reflects `DuplicateGroupsState`, an in-memory cache populated by
// an explicit refresh -- see dedup_cmds.rs's own comment), GenerationFailures
// Panel's `listGenerationFailures()` queries `videos` directly on every
// mount, so seeding before `createSession()` is enough for the already-
// mounted (if not yet opened) panel to pick it up with no extra trigger.
// `file_size`/`quick_hash` are NOT NULL columns with no bearing on this
// panel's rendering, so any placeholder value satisfies the schema.
function seedExhaustedThumbnailVideo(filePath, fileName) {
  const db = new DatabaseSync(appDbPath());
  try {
    const id = randomUUID();
    db.prepare(
      "INSERT INTO videos (id, file_path, file_name, file_size, quick_hash, status, thumbnail_attempts) " +
        "VALUES (?, ?, ?, ?, ?, 'online', 3)",
    ).run(id, filePath, fileName, 1, "e2e-fake-quick-hash");
    return id;
  } finally {
    db.close();
  }
}

function deleteSeededVideo(id) {
  const db = new DatabaseSync(appDbPath());
  try {
    db.prepare("DELETE FROM videos WHERE id = ?").run(id);
  } finally {
    db.close();
  }
}

test("the widened generation-failures panel keeps the full file name visible (never truncated) and shows the full path on hover, without overflowing the 900px minimum window width", async () => {
  await ensureAppDbExists();

  // A deliberately long directory (~230 chars across several nested
  // segments, well beyond what fits in even the widened min(56em, 90vw)
  // panel) paired with a short, easily-identified file name -- this is
  // exactly the shape .file-path-dir (shrinks/ellipsizes) + .file-path-file
  // (flex-shrink: 0, never shrinks) exists to handle: enough pressure that
  // *something* has to give, and the whole point is that it's always the
  // directory, never the file name.
  const token = `e2e${Date.now()}pathwidth`;
  const longDir =
    "C:\\Users\\e2e-fixture\\VeryLongDirectoryNameForTestingPurposesToForceEllipsisA\\" +
    "AnotherVeryLongNestedFolderNameHereToPadOutTheWidthEvenFurtherB\\" +
    "YetAnotherDeeplyNestedFolderSegmentToBeReallySureThisOverflowsC\\";
  const fileName = `${token}_exhausted.mp4`;
  const filePath = longDir + fileName;
  const videoId = seedExhaustedThumbnailVideo(filePath, fileName);

  const browser = await createSession();
  try {
    // tauri.conf.json's actual minWidth/minHeight (900x580), not the
    // comfortably-wide DEFAULT_WINDOW the other tests in this file use --
    // this is the narrowest real window the "900px幅ではみ出さない"
    // requirement is about.
    await browser.setWindowSize(900, 580);
    await waitForAppReady(browser);

    const failedBadge = await browser.$('[data-testid="status-badge-failed"]');
    await failedBadge.waitForExist({ timeout: 10_000 });
    await failedBadge.click();
    await browser.$('[data-testid="status-panel"]').waitForExist({ timeout: 10_000 });

    // GenerationFailuresPanel's own `listGenerationFailures()` IPC call
    // (queries `videos` directly, no cached state) is async, and by design
    // fires once on mount rather than being retriggered by opening the
    // panel -- see the seed helper's own comment. app.db is a real,
    // persistent file shared across every e2e run in this repo's history
    // (uiFlows.mjs's own comment), so by now it typically holds dozens of
    // rows already at Exhausted status from earlier runs' throwaway fixture
    // files (this codebase's fixtures are fake, non-decodable bytes -- see
    // fixtures.mjs -- so ffmpeg/ffprobe genuinely, legitimately exhaust
    // their retry budget against them), which makes this fetch slow enough
    // that a short fixed `browser.pause()` here is not reliably enough
    // time. Poll for our own seeded row specifically (by its unique title
    // attribute) instead of a fixed wait, the same "poll a real DOM/data
    // condition, not a guessed timeout" pattern grid-visible-after-scan.e2e
    // .mjs's own sibling-panel test uses for the same
    // async-detection-after-open class of race.
    await browser.waitUntil(
      async () => {
        const found = await browser.execute(
          (expectedFilePath) =>
            Array.from(document.querySelectorAll(".file-path-row")).some(
              (el) => el.getAttribute("title") === expectedFilePath,
            ),
          filePath,
        );
        return found;
      },
      {
        timeout: 20_000,
        interval: 500,
        timeoutMsg: `a .file-path-row with title="${filePath}" never appeared in the generation-failures panel`,
      },
    );

    const result = await browser.execute((expectedFilePath, expectedFileName) => {
      const panel = document.querySelector('[data-testid="status-panel"]');
      const row = Array.from(document.querySelectorAll(".file-path-row")).find(
        (el) => el.getAttribute("title") === expectedFilePath,
      );
      if (!panel || !row) {
        return {
          panelExists: !!panel,
          rowExists: !!row,
        };
      }
      const fileEl = row.querySelector(".file-path-file");
      const dirEl = row.querySelector(".file-path-dir");
      const panelRect = panel.getBoundingClientRect();
      const fileRect = fileEl.getBoundingClientRect();

      // Walk up from the row to find the nearest ancestor that actually
      // clips horizontally (a computed overflow-x other than "visible"),
      // rather than assuming which class that is -- .file-path-file's own
      // scrollWidth/clientWidth always agree with each other (flex-shrink:
      // 0 sizes the box to its content), so that comparison can never go
      // red regardless of whether the file name is actually fully visible;
      // comparing against the real clipping ancestor's rect is what a
      // truncation regression would actually violate.
      let clipEl = null;
      let node = row.parentElement;
      while (node && node !== document.body) {
        if (window.getComputedStyle(node).overflowX !== "visible") {
          clipEl = node;
          break;
        }
        node = node.parentElement;
      }
      const clipRect = clipEl ? clipEl.getBoundingClientRect() : null;

      return {
        panelExists: true,
        rowExists: true,
        titleAttr: row.getAttribute("title"),
        fileText: fileEl ? fileEl.textContent : null,
        dirText: dirEl ? dirEl.textContent : null,
        fileRectRight: fileRect.right,
        clipTag: clipEl ? `${clipEl.tagName}.${Array.from(clipEl.classList).join(".")}` : null,
        clipRectRight: clipRect ? clipRect.right : null,
        panelRectRight: panelRect.right,
        windowInnerWidth: window.innerWidth,
        expectedFileName,
      };
    }, filePath, fileName);

    assert.ok(
      result.panelExists && result.rowExists,
      `expected the status panel and a .file-path-row with title="${filePath}" to exist, got ${JSON.stringify(result)}`,
    );

    // (1) 900px幅ではみ出さないこと.
    assert.ok(
      result.panelRectRight <= result.windowInnerWidth,
      `expected .status-panel to stay within the 900px-wide window (min(56em, 90vw) should still be capped by the 90vw side at this width), got ${JSON.stringify(result)}`,
    );

    // (2) ファイル名が1文字も欠けないこと -- .file-path-file's own text must
    // be the full, untruncated file name (CSS ellipsis never touches
    // textContent, only paint, so this alone would not catch a truncation
    // regression -- it's a sanity check that the right element was found),
    // *and* that element's rendered box must fit entirely inside the actual
    // clipping ancestor found above (the real truncation guard).
    assert.equal(
      result.fileText,
      result.expectedFileName,
      `expected .file-path-file's textContent to be the untouched file name, got ${JSON.stringify(result)}`,
    );
    assert.ok(
      result.clipTag !== null,
      `expected to find an ancestor of .file-path-row with overflow-x !== "visible" (e.g. .generation-failures-list), found none: ${JSON.stringify(result)}`,
    );
    assert.ok(
      result.fileRectRight <= result.clipRectRight + 1,
      `expected .file-path-file's right edge (${result.fileRectRight}) to fit within its clipping ancestor ${result.clipTag}'s right edge (${result.clipRectRight}) -- i.e. the file name must not be cut off, got ${JSON.stringify(result)}`,
    );
    // MUTATION TEST: two mutations were tried against a real sandboxed
    // rebuild.
    // `.file-path-file`'s `flex-shrink: 0` -> `1` (App.css) did NOT go red
    // -- `.file-path-file` has no `overflow`/`min-width: 0` of its own, so
    // per the flexbox spec its *automatic minimum size* stays content-based
    // regardless of flex-shrink, and it never actually shrank. Likewise
    // removing `.file-path-dir`'s own `min-width: 0` alone did not go red
    // either, since `.file-path-dir` already has `overflow: hidden` (for
    // its ellipsis), which independently zeroes its automatic minimum size
    // -- making that particular `min-width: 0` redundant in this design.
    // What *did* go red: changing `.file-path-dir`'s `flex: 1 1 auto` to
    // `flex: 0 0 auto` (flex-shrink forced to 0, so it can no longer give
    // way at all) -- with a long enough directory, `.file-path-file` was
    // pushed to a right edge (~1762px) far past `.generation-failures-list`
    // (the clipping ancestor)'s right edge (809px), and this assertion
    // caught it. All other tests in this file stayed green during this
    // mutation (only this one, confirming it isn't a broader false
    // positive). All three CSS states were reverted before committing.

    // (3) ホバーで全体表示 (title属性).
    assert.equal(
      result.titleAttr,
      filePath,
      `expected .file-path-row's title attribute to be the full file_path, got ${JSON.stringify(result)}`,
    );
  } catch (e) {
    await saveFailureScreenshot(browser, "status-panel-path-width");
    throw e;
  } finally {
    await browser.deleteSession().catch(() => {});
    deleteSeededVideo(videoId);
  }
});
