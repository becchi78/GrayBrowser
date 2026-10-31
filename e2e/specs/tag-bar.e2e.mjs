// E2E flow: the left-pane TagBar -- the saved tag order (tags.sort_order)
// being what the bar shows and staying put while selecting/deselecting,
// "すべて解除"'s fixed leading position, and the full tag list with
// click-to-filter and clear-all.
//
// Tags and their order are seeded directly via DB (fixtures.mjs's
// seedTags/seedTagOrder), the same "skip the native-adjacent flow,
// write the row the real UI would otherwise produce" shape as
// seedWatchFolder -- see that function's own comment. `tags.sort_order`
// is global to the tag library (unlike a per-video tag assignment,
// which is scoped to that video and harmless to leave behind -- see
// tag-management.e2e.mjs's "e2e-smoke-tag", never cleaned up), so every
// test below restores whatever was persisted before it ran, in its own
// `finally`, to avoid leaking a tag order into whichever e2e spec runs next
// against this same, persistent app.db.
import assert from "node:assert/strict";
import { test } from "node:test";
import {
  getTagSortOrders,
  restoreTagSortOrders,
  seedTagOrder,
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

test("tag chips follow the saved order and keep their position while being selected and deselected", async () => {
  await ensureAppDbExists();

  const token = `e2e${Date.now()}ord`;
  // Deliberately not name order, so passing proves sort_order is what decides.
  const names = [`${token}_charlie`, `${token}_alpha`, `${token}_bravo`];
  const ids = seedTags(names);
  const originalOrders = getTagSortOrders();
  seedTagOrder(ids);

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
      `expected tag chips to render in the persisted sort_order, got ${JSON.stringify(initialOrder)}`,
    );

    // The order is purely the saved one -- selecting/deselecting a tag must
    // never move it.
    const bravoName = names[2];
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
          "clicking the chip never marked it aria-pressed=true",
      },
    );
    assert.deepEqual(
      await tagBarChipNames(browser, token),
      names,
      "selecting a tag must not change the chip order",
    );

    await bravoChip.click();
    await browser.waitUntil(
      async () => (await bravoChip.getAttribute("aria-pressed")) === "false",
      {
        timeout: 10_000,
        timeoutMsg: "clicking the chip again never cleared aria-pressed",
      },
    );
    assert.deepEqual(
      await tagBarChipNames(browser, token),
      names,
      "deselecting a tag must not change the chip order either",
    );
  } catch (e) {
    await saveFailureScreenshot(browser, "tag-bar-order-position");
    throw e;
  } finally {
    await browser.deleteSession().catch(() => {});
    restoreTagSortOrders(originalOrders);
  }
});

test('"すべて解除" stays the first element of the bar, disabled, with nothing selected', async () => {
  await ensureAppDbExists();

  const token = `e2e${Date.now()}clearall`;
  // TagBar renders nothing at all (`null`) once `allTags.length === 0` (see
  // its own comment) -- a tag must exist in the library for it to render
  // anything.
  seedTags([`${token}_solo`]);
  const originalOrders = getTagSortOrders();

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
    restoreTagSortOrders(originalOrders);
  }
});

test("all tags are listed (ordered ones first), toggle on click, and clear-all deselects", async () => {
  await ensureAppDbExists();

  const token = `e2e${Date.now()}others`;
  const names = [`${token}_aaa_unordered`, `${token}_zzz_ordered`];
  const ids = seedTags(names);
  const originalOrders = getTagSortOrders();
  // Only the second (name-wise later) tag is ordered; it must still come
  // before the unordered one.
  seedTagOrder([ids[1]]);

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
      "the ordered tag must come before the unordered one, and both must be listed",
    );

    await otherChip.click();
    await browser.waitUntil(
      async () => (await otherChip.getAttribute("aria-pressed")) === "true",
      {
        timeout: 10_000,
        timeoutMsg:
          "clicking an unordered tag never marked it aria-pressed=true",
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
    restoreTagSortOrders(originalOrders);
  }
});
