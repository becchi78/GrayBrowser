// E2E flow: the left-pane TagBar -- pinned-tag position stability,
// "すべて解除"'s fixed leading position, and the full tag list (pinned first,
// then the rest) with click-to-filter and clear-all.
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
import {
  getTagBarPinnedTagIds,
  seedTagBarPinnedTagIds,
  seedTags,
} from "../fixtures.mjs";
import { createSession, saveFailureScreenshot } from "../session.mjs";
import { ensureAppDbExists, waitForAppReady } from "../uiFlows.mjs";

const DEFAULT_WINDOW = { width: 1280, height: 900 };

// The list shows every tag in the library, so callers filter by their own
// seeded names (token-prefixed) to stay independent of leftover tags.
async function tagBarChipNames(browser, token = "") {
  const all = await browser.execute(() =>
    Array.from(document.querySelectorAll('[data-testid="tag-bar-chip"]')).map(
      (el) => el.textContent,
    ),
  );
  return all.filter((name) => name.startsWith(token));
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
    await browser.waitUntil(
      async () =>
        (await tagBarChipNames(browser, token)).length === names.length,
      {
        timeout: 15_000,
        interval: 300,
        timeoutMsg: "seeded tag chips never appeared",
      },
    );

    const initialOrder = await tagBarChipNames(browser, token);
    assert.deepEqual(
      initialOrder,
      names,
      `expected pinned tag chips to render in the persisted pinned order, got ${JSON.stringify(initialOrder)}`,
    );

    // The middle pinned tag ("bravo") is already pinned before this click --
    // per TagBar.tsx's own design, selecting/deselecting an *already-pinned*
    // tag must never move it.
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
        timeoutMsg:
          "clicking the pinned chip never marked it aria-pressed=true",
      },
    );
    assert.deepEqual(
      await tagBarChipNames(browser, token),
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
      await tagBarChipNames(browser, token),
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
      const firstButton = document.querySelector(
        '[data-testid="tag-bar"] button',
      );
      return firstButton?.getAttribute("data-testid") === "tag-bar-clear-all";
    });
    assert.ok(
      isFirstButton,
      '"すべて解除" must be the first button rendered inside .tag-bar',
    );

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

test("all tags are listed (pinned first), toggle on click, and clear-all deselects", async () => {
  await ensureAppDbExists();

  const token = `e2e${Date.now()}others`;
  const names = [`${token}_pinned`, `${token}_other`];
  const ids = seedTags(names);
  const originalPinned = getTagBarPinnedTagIds();
  seedTagBarPinnedTagIds([ids[1]]);

  const browser = await createSession();
  try {
    await browser.setWindowSize(DEFAULT_WINDOW.width, DEFAULT_WINDOW.height);
    await waitForAppReady(browser);

    const otherChip = await browser.$(
      `//button[@data-testid="tag-bar-chip" and text()="${names[0]}"]`,
    );
    await otherChip.waitForDisplayed({ timeout: 10_000 });
    const listed = await tagBarChipNames(browser, token);
    assert.ok(
      listed.indexOf(names[1]) !== -1 &&
        listed.indexOf(names[1]) < listed.indexOf(names[0]),
      "the pinned tag must come before the unpinned one, and both must be listed",
    );

    await otherChip.click();
    await browser.waitUntil(
      async () => (await otherChip.getAttribute("aria-pressed")) === "true",
      {
        timeout: 10_000,
        timeoutMsg:
          "clicking an unpinned tag never marked it aria-pressed=true",
      },
    );
    assert.deepEqual(
      await tagBarChipNames(browser, token),
      listed,
      "selecting a tag must not reorder the list",
    );

    const clearAll = await browser.$('[data-testid="tag-bar-clear-all"]');
    await clearAll.click();
    await browser.waitUntil(
      async () => (await otherChip.getAttribute("aria-pressed")) === "false",
      { timeout: 10_000, timeoutMsg: "clear-all never deselected the tag" },
    );
  } catch (e) {
    await saveFailureScreenshot(browser, "tag-bar-others");
    throw e;
  } finally {
    await browser.deleteSession().catch(() => {});
    seedTagBarPinnedTagIds(originalPinned);
  }
});
