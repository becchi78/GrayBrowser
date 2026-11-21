import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  clampMenuPosition,
  contextMenuAction,
  placeDropdownRightAligned,
  placeSubmenu,
} from "./contextMenu.ts";

describe("contextMenuAction", () => {
  it("suppresses on ordinary UI in a production build", () => {
    assert.equal(contextMenuAction({ isTextEdit: false, isDev: false }), "suppress");
  });

  it("shows the app edit menu on text fields in a production build", () => {
    assert.equal(contextMenuAction({ isTextEdit: true, isDev: false }), "edit-menu");
  });

  it("keeps the native menu in dev builds for Inspect", () => {
    assert.equal(contextMenuAction({ isTextEdit: false, isDev: true }), "native");
  });

  it("keeps the native menu in dev even on text fields", () => {
    assert.equal(contextMenuAction({ isTextEdit: true, isDev: true }), "native");
  });
});

const viewport = { width: 1000, height: 600 };
const menu = { width: 200, height: 40 };

describe("clampMenuPosition", () => {
  it("leaves a menu that fits where the pointer is", () => {
    assert.deepEqual(clampMenuPosition({ x: 100, y: 100 }, menu, viewport), {
      x: 100,
      y: 100,
    });
  });

  it("shifts left when it would overflow the right edge", () => {
    assert.deepEqual(clampMenuPosition({ x: 950, y: 100 }, menu, viewport), {
      x: 796,
      y: 100,
    });
  });

  it("shifts up when it would overflow the bottom edge", () => {
    assert.deepEqual(clampMenuPosition({ x: 100, y: 590 }, menu, viewport), {
      x: 100,
      y: 556,
    });
  });

  it("handles the bottom-right corner", () => {
    assert.deepEqual(clampMenuPosition({ x: 1000, y: 600 }, menu, viewport), {
      x: 796,
      y: 556,
    });
  });

  it("allows a menu that exactly fits against the edge margin", () => {
    assert.deepEqual(clampMenuPosition({ x: 796, y: 556 }, menu, viewport), {
      x: 796,
      y: 556,
    });
  });

  it("never goes past the top-left margin, even for a menu larger than the viewport", () => {
    assert.deepEqual(clampMenuPosition({ x: 10, y: 10 }, { width: 2000, height: 900 }, viewport), {
      x: 4,
      y: 4,
    });
    assert.deepEqual(clampMenuPosition({ x: -5, y: -5 }, menu, viewport), {
      x: 4,
      y: 4,
    });
  });
});

describe("placeDropdownRightAligned", () => {
  const viewport = { width: 1000, height: 600 };
  const menu = { width: 200, height: 150 };

  it("lines the right edges up, directly below the anchor", () => {
    assert.deepEqual(placeDropdownRightAligned({ right: 980, bottom: 40 }, menu, viewport), {
      x: 780,
      y: 44,
    });
  });

  it("is shifted inward when the left edge would leave the window", () => {
    assert.equal(placeDropdownRightAligned({ right: 90, bottom: 40 }, menu, viewport).x, 4);
  });

  it("is shifted up when it would overflow the bottom", () => {
    assert.equal(placeDropdownRightAligned({ right: 980, bottom: 550 }, menu, viewport).y, 446);
  });
});

describe("placeSubmenu", () => {
  const viewport = { width: 1000, height: 600 };

  it("opens on the left of the parent, flush against its left edge", () => {
    const p = placeSubmenu({ left: 780, right: 980 }, 96, { width: 160, height: 100 }, viewport);
    assert.deepEqual(p, { x: 620, y: 96, side: "left" });
  });

  it("opens on the right when the left side does not fit", () => {
    const p = placeSubmenu({ left: 100, right: 300 }, 96, { width: 160, height: 100 }, viewport);
    assert.deepEqual(p, { x: 300, y: 96, side: "right" });
  });

  it("uses the left side when it fits exactly with the margin", () => {
    const p = placeSubmenu({ left: 164, right: 300 }, 10, { width: 160, height: 100 }, viewport);
    assert.equal(p.side, "left");
    assert.equal(p.x, 4);
  });

  it("is shifted up when it would overflow the bottom, and down from the top edge", () => {
    const low = placeSubmenu({ left: 780, right: 980 }, 560, { width: 160, height: 100 }, viewport);
    assert.equal(low.y, 496);
    const high = placeSubmenu({ left: 780, right: 980 }, -20, { width: 160, height: 100 }, viewport);
    assert.equal(high.y, 4);
  });

  it("slides back inside the window on the right side, overlapping the parent, when neither side has room", () => {
    const p = placeSubmenu({ left: 20, right: 150 }, 10, { width: 200, height: 50 }, { width: 300, height: 600 });
    assert.equal(p.side, "right");
    assert.equal(p.x, 96);
  });
});
