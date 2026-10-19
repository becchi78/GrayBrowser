import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  clearSelection,
  clickSelect,
  pruneSelection,
  selectAll,
  type SelectionState,
} from "./multiSelect.ts";

const order = ["a", "b", "c", "d", "e"];
const none = { ctrl: false, shift: false };
const ctrl = { ctrl: true, shift: false };
const shift = { ctrl: false, shift: true };
const ctrlShift = { ctrl: true, shift: true };

function sel(ids: string[], anchorId: string | null): SelectionState {
  return { ids: new Set(ids), anchorId };
}

function sorted(s: SelectionState): string[] {
  return [...s.ids].sort();
}

describe("clickSelect", () => {
  it("plain click selects only the clicked row and sets the anchor", () => {
    const s = clickSelect(sel(["a", "b"], "a"), order, "c", none);
    assert.deepEqual(sorted(s), ["c"]);
    assert.equal(s.anchorId, "c");
  });

  it("plain click on the only selected row keeps it selected", () => {
    const s = clickSelect(sel(["c"], "c"), order, "c", none);
    assert.deepEqual(sorted(s), ["c"]);
  });

  it("ctrl click adds and removes, moving the anchor", () => {
    const added = clickSelect(sel(["a"], "a"), order, "c", ctrl);
    assert.deepEqual(sorted(added), ["a", "c"]);
    assert.equal(added.anchorId, "c");
    const removed = clickSelect(added, order, "a", ctrl);
    assert.deepEqual(sorted(removed), ["c"]);
    assert.equal(removed.anchorId, "a");
  });

  it("shift click selects the range from the anchor and keeps the anchor", () => {
    const s = clickSelect(sel(["b"], "b"), order, "d", shift);
    assert.deepEqual(sorted(s), ["b", "c", "d"]);
    assert.equal(s.anchorId, "b");
  });

  it("shift click works in the reverse direction", () => {
    const s = clickSelect(sel(["d"], "d"), order, "b", shift);
    assert.deepEqual(sorted(s), ["b", "c", "d"]);
  });

  it("shift click replaces the previous selection", () => {
    const s = clickSelect(sel(["a", "e"], "b"), order, "c", shift);
    assert.deepEqual(sorted(s), ["b", "c"]);
  });

  it("shift click on the anchor selects just that row", () => {
    const s = clickSelect(sel(["b", "c"], "b"), order, "b", shift);
    assert.deepEqual(sorted(s), ["b"]);
  });

  it("ctrl+shift adds the range to the existing selection", () => {
    const s = clickSelect(sel(["a", "c"], "c"), order, "e", ctrlShift);
    assert.deepEqual(sorted(s), ["a", "c", "d", "e"]);
    assert.equal(s.anchorId, "c");
  });

  it("shift without an anchor behaves as a plain click", () => {
    const s = clickSelect(clearSelection(), order, "c", shift);
    assert.deepEqual(sorted(s), ["c"]);
    assert.equal(s.anchorId, "c");
  });

  it("ctrl+shift without an anchor behaves as a ctrl click", () => {
    const s = clickSelect(sel(["a"], null), order, "c", ctrlShift);
    assert.deepEqual(sorted(s), ["a", "c"]);
    assert.equal(s.anchorId, "c");
  });

  it("shift with an anchor no longer in the list behaves as a plain click", () => {
    const s = clickSelect(sel(["z"], "z"), order, "c", shift);
    assert.deepEqual(sorted(s), ["c"]);
  });

  it("does not mutate the input state", () => {
    const before = sel(["a"], "a");
    clickSelect(before, order, "b", ctrl);
    assert.deepEqual(sorted(before), ["a"]);
    assert.equal(before.anchorId, "a");
  });
});

describe("selectAll / clearSelection", () => {
  it("selectAll selects every id and has no anchor", () => {
    const s = selectAll(order);
    assert.deepEqual(sorted(s), order);
    assert.equal(s.anchorId, null);
  });

  it("selectAll on an empty list is empty", () => {
    assert.equal(selectAll([]).ids.size, 0);
  });

  it("clearSelection is empty with no anchor", () => {
    const s = clearSelection();
    assert.equal(s.ids.size, 0);
    assert.equal(s.anchorId, null);
  });
});

describe("pruneSelection", () => {
  it("drops ids that disappeared from the list", () => {
    const s = pruneSelection(sel(["a", "b", "c"], "a"), ["a", "c"]);
    assert.deepEqual(sorted(s), ["a", "c"]);
    assert.equal(s.anchorId, "a");
  });

  it("clears the anchor when it disappeared", () => {
    const s = pruneSelection(sel(["a", "b"], "a"), ["b", "c"]);
    assert.deepEqual(sorted(s), ["b"]);
    assert.equal(s.anchorId, null);
  });

  it("empties the selection when the list is empty", () => {
    const s = pruneSelection(sel(["a"], "a"), []);
    assert.equal(s.ids.size, 0);
    assert.equal(s.anchorId, null);
  });

  it("does not select ids that were never selected", () => {
    const s = pruneSelection(sel(["a"], "a"), order);
    assert.deepEqual(sorted(s), ["a"]);
  });
});
