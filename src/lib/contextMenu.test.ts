import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { clampMenuPosition } from "./contextMenu.ts";

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
