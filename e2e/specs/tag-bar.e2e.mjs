// E2E flow: the TagBar (Phase A-D) -- pinned-tag position stability,
// "すべて解除"'s fixed leading position, and the ▾ overflow dropdown.
//
// Tags/pinned list are seeded directly via DB (fixtures.mjs's
// seedTags/seedTagBarPinnedTagIds), the same "skip the native-adjacent flow,
// write the row the real UI would otherwise produce" shape as
// seedWatchFolder -- see that function's own comment. `tag_bar_pinned_tags`
// is a single global `settings` row (unlike a per-video tag assignment,
// which is scoped to that video and harmless to leave behind -- see
// tag-management.e2e.mjs's "e2e-smoke-tag", never cleaned up), so every
// test below restores whatever was persisted before it ran, in its own
// `finally`, to avoid leaking pinned tags into whichever e2e spec runs next
// against this same, persistent app.db.
import assert from "node:assert/strict";
import { test } from "node:test";
import { getTagBarPinnedTagIds, seedTagBarPinnedTagIds, seedTags } from "../fixtures.mjs";
import { createSession, saveFailureScreenshot } from "../session.mjs";
import { ensureAppDbExists, waitForAppReady } from "../uiFlows.mjs";

const DEFAULT_WINDOW = { width: 1280, height: 900 };

async function waitForTagBarChipCount(browser, expectedCount) {
  await browser.waitUntil(
    async () => (await browser.$$('[data-testid="tag-bar-chip"]')).length === expectedCount,
    {
      timeout: 15_000,
      interval: 300,
      timeoutMsg: `tag-bar-chip count never reached ${expectedCount}`,
    },
  );
}

async function tagBarChipNames(browser) {
  return browser.execute(() =>
    Array.from(document.querySelectorAll('[data-testid="tag-bar-chip"]')).map(
      (el) => el.textContent,
    ),
  );
}

test("pinned tag chips keep their bar position while being selected and deselected", async () => {
  await ensureAppDbExists();

  const token = `e2e${Date.now()}pin`;
  const names = [`${token}_alpha`, `${token}_bravo`, `${token}_charlie`];
  const ids = seedTags(names);
  const originalPinned = getTagBarPinnedTagIds();
  seedTagBarPinnedTagIds(ids);

  const browser = await createSession();
  try {
    await browser.setWindowSize(DEFAULT_WINDOW.width, DEFAULT_WINDOW.height);
    await waitForAppReady(browser);
    await waitForTagBarChipCount(browser, names.length);

    const initialOrder = await tagBarChipNames(browser);
    assert.deepEqual(
      initialOrder,
      names,
      `expected pinned tag chips to render in the persisted pinned order, got ${JSON.stringify(initialOrder)}`,
    );

    // The middle pinned tag ("bravo") is already pinned before this click --
    // per TagBar.tsx's own design, selecting/deselecting an *already-pinned*
    // tag must never move it (unlike a newly-selected, not-yet-pinned tag,
    // which would appear as a "promoted" chip -- out of scope for this
    // fixed-position assertion).
    const bravoName = names[1];
    const bravoChip = await browser.$(
      `//button[@data-testid="tag-bar-chip" and text()="${bravoName}"]`,
    );
    await bravoChip.waitForExist({ timeout: 10_000 });

    await bravoChip.click();
    await browser.waitUntil(
      async () => (await bravoChip.getAttribute("aria-pressed")) === "true",
      {
        timeout: 10_000,
        timeoutMsg: "clicking the pinned chip never marked it aria-pressed=true",
      },
    );
    assert.deepEqual(
      await tagBarChipNames(browser),
      names,
      "selecting an already-pinned tag must not change the chip order",
    );

    await bravoChip.click();
    await browser.waitUntil(
      async () => (await bravoChip.getAttribute("aria-pressed")) === "false",
      {
        timeout: 10_000,
        timeoutMsg: "clicking the pinned chip again never cleared aria-pressed",
      },
    );
    assert.deepEqual(
      await tagBarChipNames(browser),
      names,
      "deselecting an already-pinned tag must not change the chip order either",
    );
  } catch (e) {
    await saveFailureScreenshot(browser, "tag-bar-pinned-position");
    throw e;
  } finally {
    await browser.deleteSession().catch(() => {});
    seedTagBarPinnedTagIds(originalPinned);
  }
});

test('"すべて解除" stays the first element of the bar, disabled, with nothing selected or pinned', async () => {
  await ensureAppDbExists();

  const token = `e2e${Date.now()}clearall`;
  // TagBar renders nothing at all (`null`) once `allTags.length === 0` (see
  // its own comment) -- a tag must exist in the library for it to render
  // anything, but this scenario is about the "0件選択・0件pin" state
  // specifically, so the seeded tag is deliberately left unpinned.
  seedTags([`${token}_solo`]);
  const originalPinned = getTagBarPinnedTagIds();
  seedTagBarPinnedTagIds([]);

  const browser = await createSession();
  try {
    await browser.setWindowSize(DEFAULT_WINDOW.width, DEFAULT_WINDOW.height);
    await waitForAppReady(browser);

    const clearAll = await browser.$('[data-testid="tag-bar-clear-all"]');
    await clearAll.waitForExist({ timeout: 10_000 });

    // Checks the first `<button>` descendant in document order (not
    // `.tag-bar`'s own `firstElementChild`) -- deliberately
    // structure-agnostic to `.tag-bar`'s internal wrapper elements (e.g.
    // `.tag-bar-row`, added to scope the chip-row's own `overflow: hidden`
    // clip away from the ▾ dropdown popover -- see App.css's own comment).
    // `.tag-bar` is a plain `display: flex` row (no `flex-direction:
    // row-reverse`/`order`), so document order and left-to-right visual
    // order still coincide here.
    const isFirstButton = await browser.execute(() => {
      const firstButton = document.querySelector('[data-testid="tag-bar"] button');
      return firstButton?.getAttribute("data-testid") === "tag-bar-clear-all";
    });
    assert.ok(isFirstButton, '"すべて解除" must be the first button rendered inside .tag-bar');

    assert.equal(
      await clearAll.isEnabled(),
      false,
      '"すべて解除" must be disabled while nothing is selected (star-clear-style, see StarRating.tsx)',
    );
  } catch (e) {
    await saveFailureScreenshot(browser, "tag-bar-clear-all-position");
    throw e;
  } finally {
    await browser.deleteSession().catch(() => {});
    seedTagBarPinnedTagIds(originalPinned);
  }
});

test("a narrow window routes pinned tags that don't fit into the ▾ overflow dropdown", async () => {
  await ensureAppDbExists();

  // 15 long-enough names that `.tag-bar-chip`'s own `max-width: 12em` clamp
  // (App.css) kicks in on every one of them -- their *rendered* (clamped)
  // width is what TagBar.tsx's hidden measurement row actually measures, so
  // padding them out further than 12em worth of characters would be
  // pointless; going past the clamp just guarantees each one measures at
  // its full 12em ceiling rather than depending on font-metric specifics.
  const token = `e2e${Date.now()}ovf`;
  const names = Array.from(
    { length: 15 },
    (_, i) => `${token}_overflow_candidate_number_${i}_long_enough_to_clamp`,
  );
  const ids = seedTags(names);
  const originalPinned = getTagBarPinnedTagIds();
  seedTagBarPinnedTagIds(ids);

  const browser = await createSession();
  try {
    // tauri.conf.json's real minWidth (900x580) -- a genuinely reachable
    // window size (unlike layout-shrink-regression.e2e.mjs's sub-minWidth
    // SHRUNK_WIDTH trick), still narrow enough that 15 max-width-clamped
    // chips can't possibly all fit alongside "すべて解除".
    await browser.setWindowSize(900, 580);
    await waitForAppReady(browser);
    await browser.pause(500);

    const barChipNames = await tagBarChipNames(browser);
    assert.ok(
      barChipNames.length > 0 && barChipNames.length < names.length,
      `expected some (but not all) of the ${names.length} pinned chips to fit in the bar at 900px, got ${barChipNames.length}: ${JSON.stringify(barChipNames)}`,
    );
    // computeVisibleChipCount always keeps the *first* N candidates (in
    // order) and routes the rest to the dropdown -- never a scattered
    // subset -- so whatever fit must be an exact prefix of the pinned order.
    assert.deepEqual(
      barChipNames,
      names.slice(0, barChipNames.length),
      `expected the chips that fit to be an exact prefix of the pinned order, got ${JSON.stringify(barChipNames)}`,
    );

    const overflowToggle = await browser.$('[data-testid="tag-bar-overflow-toggle"]');
    await overflowToggle.waitForExist({ timeout: 10_000 });
    await overflowToggle.click();

    // `waitForExist()` alone is exactly what missed a real bug here: the
    // panel was correctly present in the DOM (React's `overflowOpen` state
    // toggled fine) while still invisible on screen, because `.tag-bar`'s
    // own `overflow: hidden` (meant only to clip an over-budget chip row)
    // clipped this panel's `position: absolute; top: 100%` popover too --
    // CSS clips a descendant to its clipping ancestor's box regardless of
    // the descendant's own `position` (see TagBar.tsx/App.css's own
    // comments on `.tag-bar`/`.tag-bar-row` for the fix).
    const overflowPanel = await browser.$('[data-testid="tag-bar-overflow-panel"]');
    await overflowPanel.waitForExist({ timeout: 10_000 });
    // A necessary but NOT sufficient check for this specific bug class --
    // kept as a basic sanity gate (still catches an unrelated `display:
    // none`/`visibility: hidden`/`opacity: 0` regression), but empirically
    // confirmed (a temporary, reverted mutation re-adding `overflow: hidden`
    // to `.tag-bar` itself, then rebuilding against a real sandboxed app)
    // that it stays `true` even while the panel is fully clipped out of view
    // by an ancestor's `overflow: hidden` box: WebDriver's "is element
    // displayed" check only looks at computed `display`/`visibility`/
    // `opacity`, never whether an ancestor's overflow clips the element's
    // box out of the visible area. The `clipCheck` geometric assertion below
    // is what actually guards against *this* bug.
    await overflowPanel.waitForDisplayed({ timeout: 10_000 });

    // Asserts something `isDisplayed()` can't: that the panel isn't (even
    // partially) clipped away by the nearest real clipping ancestor found by
    // walking up the DOM -- the same "find the real clipping ancestor,
    // don't assume which class it is" pattern
    // layout-shrink-regression.e2e.mjs's generation-failures-panel test
    // already uses for the same reason.
    const clipCheck = await browser.execute(() => {
      const panel = document.querySelector('[data-testid="tag-bar-overflow-panel"]');
      const panelRect = panel.getBoundingClientRect();
      let clipEl = null;
      let node = panel.parentElement;
      while (node && node !== document.body) {
        const style = window.getComputedStyle(node);
        if (style.overflowX !== "visible" || style.overflowY !== "visible") {
          clipEl = node;
          break;
        }
        node = node.parentElement;
      }
      const clipRect = clipEl ? clipEl.getBoundingClientRect() : null;
      return {
        panelRect: {
          top: panelRect.top,
          bottom: panelRect.bottom,
          left: panelRect.left,
          right: panelRect.right,
        },
        clipTag: clipEl ? `${clipEl.tagName}.${Array.from(clipEl.classList).join(".")}` : null,
        clipRect: clipRect
          ? { top: clipRect.top, bottom: clipRect.bottom, left: clipRect.left, right: clipRect.right }
          : null,
      };
    });
    if (clipCheck.clipTag) {
      const verticalOverlap =
        Math.min(clipCheck.panelRect.bottom, clipCheck.clipRect.bottom) -
        Math.max(clipCheck.panelRect.top, clipCheck.clipRect.top);
      const horizontalOverlap =
        Math.min(clipCheck.panelRect.right, clipCheck.clipRect.right) -
        Math.max(clipCheck.panelRect.left, clipCheck.clipRect.left);
      assert.ok(
        verticalOverlap > 0 && horizontalOverlap > 0,
        `expected the ▾ dropdown panel to actually overlap its nearest clipping ancestor (${clipCheck.clipTag}), i.e. not be entirely clipped out of view -- got ${JSON.stringify(clipCheck)}`,
      );
    }

    const overflowChipElements = await browser.$$('[data-testid="tag-bar-overflow-chip"]');
    assert.ok(
      overflowChipElements.length > 0,
      "expected at least one chip inside the ▾ dropdown panel",
    );

    const overflowChipNames = await browser.execute(() =>
      Array.from(document.querySelectorAll('[data-testid="tag-bar-overflow-chip"]')).map(
        (el) => el.textContent,
      ),
    );
    const expectedOverflowNames = names.slice(barChipNames.length);
    for (const name of expectedOverflowNames) {
      assert.ok(
        overflowChipNames.includes(name),
        `expected overflowed pinned tag "${name}" to appear in the ▾ dropdown, got ${JSON.stringify(overflowChipNames)}`,
      );
    }
    // MUTATION TEST: re-adding `overflow: hidden` to `.tag-bar` itself
    // (App.css) -- i.e. reverting just the clip-scoping fix while keeping
    // everything else in this file unchanged -- and rebuilding against a
    // real sandboxed app confirmed this exact `clipCheck` assertion goes
    // red (`verticalOverlap` negative: the panel's top edge sat below
    // `.tag-bar`'s own clipped bottom edge). Also confirmed in the same run:
    // `isDisplayed()`/`waitForDisplayed()` on the panel stayed `true`
    // throughout that mutation -- i.e. those checks alone do NOT catch this
    // bug class at all (WebDriver's displayedness check doesn't model
    // ancestor-overflow clipping), which is why this test asserts the
    // geometric overlap directly instead of relying on them. Reverted
    // before finishing. The other core claims here (an exact prefix fits,
    // the rest reach the dropdown) are separately already
    // functional/behavioral, not just a measurement that could pass for the
    // wrong reason -- a `computeVisibleChipCount` regression that returned
    // e.g. always-0 or always-all would fail the `length > 0 && length <
    // names.length` assertion directly, and one that scrambled order would
    // fail the prefix-equality assertion.
  } catch (e) {
    await saveFailureScreenshot(browser, "tag-bar-overflow");
    throw e;
  } finally {
    await browser.deleteSession().catch(() => {});
    seedTagBarPinnedTagIds(originalPinned);
  }
});
