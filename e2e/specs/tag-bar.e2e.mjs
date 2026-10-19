// E2E flow: the left-pane TagBar -- pinned-tag position stability,
// "すべて解除"'s fixed leading position, and the 「その他のタグ」 list with
// promotion.
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
    // structure-agnostic to `.tag-bar`'s internal wrapper elements. `.tag-bar`
    // is a plain column (no `order`/`*-reverse`), so document order and
    // top-to-bottom visual order coincide.
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

test("tags that are not pinned are reachable from the 「その他のタグ」 list and get promoted when selected", async () => {
  await ensureAppDbExists();

  const token = `e2e${Date.now()}others`;
  const names = [`${token}_pinned`, `${token}_other`];
  const ids = seedTags(names);
  const originalPinned = getTagBarPinnedTagIds();
  seedTagBarPinnedTagIds([ids[0]]);

  const browser = await createSession();
  try {
    await browser.setWindowSize(DEFAULT_WINDOW.width, DEFAULT_WINDOW.height);
    await waitForAppReady(browser);
    await waitForTagBarChipCount(browser, 1);

    const othersToggle = await browser.$('[data-testid="tag-bar-others-toggle"]');
    await othersToggle.waitForExist({ timeout: 10_000 });
    assert.equal(
      await othersToggle.getAttribute("aria-expanded"),
      "false",
      "the 「その他のタグ」 list must start collapsed",
    );
    await othersToggle.click();

    const otherChip = await browser.$(
      `//button[@data-testid="tag-bar-others-chip" and text()="${names[1]}"]`,
    );
    await otherChip.waitForDisplayed({ timeout: 10_000 });
    assert.ok(
      !(await tagBarChipNames(browser)).includes(names[1]),
      "an unpinned, unselected tag must not appear among the primary chips",
    );

    await otherChip.click();
    // Selecting it promotes it ahead of the pinned chips.
    await waitForTagBarChipCount(browser, 2);
    assert.deepEqual(
      (await tagBarChipNames(browser)).slice(0, 2),
      [names[1], names[0]],
      "a selected, unpinned tag must be promoted before the pinned chips",
    );
  } catch (e) {
    await saveFailureScreenshot(browser, "tag-bar-others");
    throw e;
  } finally {
    await browser.deleteSession().catch(() => {});
    seedTagBarPinnedTagIds(originalPinned);
  }
});
