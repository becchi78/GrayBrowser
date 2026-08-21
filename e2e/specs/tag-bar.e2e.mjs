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

    const isFirstChild = await browser.execute(() => {
      const bar = document.querySelector('[data-testid="tag-bar"]');
      return bar?.firstElementChild?.getAttribute("data-testid") === "tag-bar-clear-all";
    });
    assert.ok(isFirstChild, '"すべて解除" must be the first child of .tag-bar');

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
    // chips can't possibly all fit alongside "すべて解除"/"編集 ▸".
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
    await browser.$('[data-testid="tag-bar-overflow-panel"]').waitForExist({ timeout: 10_000 });

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
    // Not mutation-tested here (unlike layout-shrink-regression.e2e.mjs's
    // CSS-only assertions, which need a rebuild to mutate against): this
    // test's core claims (an exact prefix fits, the rest reach the
    // dropdown) are already functional/behavioral, not just a measurement
    // that could pass for the wrong reason -- a `computeVisibleChipCount`
    // regression that returned e.g. always-0 or always-all would fail the
    // `length > 0 && length < names.length` assertion directly, and one
    // that scrambled order would fail the prefix-equality assertion.
  } catch (e) {
    await saveFailureScreenshot(browser, "tag-bar-overflow");
    throw e;
  } finally {
    await browser.deleteSession().catch(() => {});
    seedTagBarPinnedTagIds(originalPinned);
  }
});
