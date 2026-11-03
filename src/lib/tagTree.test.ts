import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  buildTagTree,
  canMove,
  displayOrder,
  hasChildren,
  moveWithinSiblings,
  parentCandidates,
  removeTag,
  setParent,
  toLayoutItems,
} from "./tagTree.ts";

const t = (id: number, parent_id: number | null = null) => ({ id, parent_id });
const ids = (tags: { id: number }[]) => tags.map((x) => x.id);
const pairs = (tags: { id: number; parent_id: number | null }[]) =>
  tags.map((x) => [x.id, x.parent_id]);

// 1 と 4 がトップレベル、2・3 は 1 の子。入力の順(SQL の並び)はばらばら。
const sample = () => [t(1), t(4), t(2, 1), t(3, 1)];

describe("buildTagTree / displayOrder", () => {
  it("puts children right after their parent, keeping sibling order", () => {
    assert.deepEqual(ids(displayOrder(sample())), [1, 2, 3, 4]);
    assert.deepEqual(ids(displayOrder([t(3, 1), t(4), t(1), t(2, 1)])), [4, 1, 3, 2]);
  });

  it("builds the tree with children under their parent", () => {
    const tree = buildTagTree(sample());
    assert.deepEqual(
      tree.map((n) => [n.tag.id, ids(n.children)]),
      [
        [1, [2, 3]],
        [4, []],
      ],
    );
  });

  it("treats a child of a missing parent as top-level", () => {
    assert.deepEqual(pairs(displayOrder([t(1, 99), t(2)])), [
      [1, null],
      [2, null],
    ]);
  });

  it("treats a third-level tag as top-level so it does not disappear", () => {
    assert.deepEqual(pairs(displayOrder([t(1), t(2, 1), t(3, 2)])), [
      [1, null],
      [2, 1],
      [3, null],
    ]);
  });

  it("survives a self parent and a cycle, listing every tag once", () => {
    const out = displayOrder([t(1, 1), t(2, 3), t(3, 2)]);
    assert.deepEqual(ids(out).sort(), [1, 2, 3]);
    assert.ok(out.every((x) => x.parent_id === null));
  });

  it("handles an empty list", () => {
    assert.deepEqual(displayOrder([]), []);
  });
});

describe("hasChildren / parentCandidates", () => {
  it("knows which tags have children", () => {
    assert.equal(hasChildren(sample(), 1), true);
    assert.equal(hasChildren(sample(), 4), false);
    assert.equal(hasChildren(sample(), 2), false);
  });

  it("offers top-level tags other than itself", () => {
    assert.deepEqual(ids(parentCandidates(sample(), 4)), [1]);
    assert.deepEqual(ids(parentCandidates([t(1), t(2), t(3)], 2)), [1, 3]);
  });

  it("offers nothing to a tag that has children", () => {
    assert.deepEqual(parentCandidates(sample(), 1), []);
  });

  it("does not offer child tags, but keeps a child's current parent", () => {
    assert.deepEqual(ids(parentCandidates(sample(), 2)), [1, 4]);
  });
});

describe("setParent", () => {
  it("moves a top-level tag under a parent as its last child", () => {
    const out = setParent(sample(), 4, 1);
    assert.deepEqual(pairs(out), [
      [1, null],
      [2, 1],
      [3, 1],
      [4, 1],
    ]);
  });

  it("clears the parent and places the tag at the end of the top level", () => {
    const out = setParent([t(1), t(2, 1), t(3, 1), t(4)], 2, null);
    assert.deepEqual(pairs(out), [
      [1, null],
      [3, 1],
      [4, null],
      [2, null],
    ]);
  });

  it("refuses a tag that has children, a child as parent, and itself", () => {
    const tags = sample();
    assert.equal(setParent(tags, 1, 4), tags);
    assert.equal(setParent(tags, 4, 2), tags);
    assert.equal(setParent(tags, 4, 4), tags);
    assert.equal(setParent(tags, 4, 99), tags);
  });

  it("is a no-op when the parent does not change", () => {
    const tags = sample();
    assert.equal(setParent(tags, 2, 1), tags);
    assert.equal(setParent(tags, 4, null), tags);
  });
});

describe("moveWithinSiblings / canMove", () => {
  it("moves a child among its siblings only", () => {
    assert.deepEqual(ids(moveWithinSiblings(sample(), 3, "up")), [1, 3, 2, 4]);
    assert.deepEqual(ids(moveWithinSiblings(sample(), 2, "down")), [1, 3, 2, 4]);
  });

  it("moves a parent together with its children", () => {
    assert.deepEqual(ids(moveWithinSiblings(sample(), 4, "up")), [4, 1, 2, 3]);
    assert.deepEqual(ids(moveWithinSiblings(sample(), 1, "down")), [4, 1, 2, 3]);
  });

  it("does not let a child leave its parent", () => {
    const tags = sample();
    assert.equal(canMove(tags, 2, "up"), false);
    assert.equal(canMove(tags, 3, "down"), false);
    assert.equal(moveWithinSiblings(tags, 2, "up"), tags);
    assert.equal(moveWithinSiblings(tags, 3, "down"), tags);
  });

  it("does not move past either end of the top level", () => {
    const tags = sample();
    assert.equal(canMove(tags, 1, "up"), false);
    assert.equal(canMove(tags, 4, "down"), false);
    assert.equal(canMove(tags, 4, "up"), true);
  });

  it("ignores an unknown tag", () => {
    const tags = sample();
    assert.equal(canMove(tags, 99, "up"), false);
    assert.equal(moveWithinSiblings(tags, 99, "down"), tags);
  });
});

describe("removeTag", () => {
  it("returns the children of a removed parent to the top level", () => {
    assert.deepEqual(pairs(removeTag(sample(), 1)), [
      [4, null],
      [2, null],
      [3, null],
    ]);
  });

  it("removes a child and keeps the rest", () => {
    assert.deepEqual(pairs(removeTag(sample(), 2)), [
      [1, null],
      [3, 1],
      [4, null],
    ]);
  });
});

describe("toLayoutItems", () => {
  it("lists parents followed by their children with the parent id", () => {
    assert.deepEqual(toLayoutItems([t(3, 1), t(4), t(1), t(2, 1)]), [
      { id: 4, parent_id: null },
      { id: 1, parent_id: null },
      { id: 3, parent_id: 1 },
      { id: 2, parent_id: 1 },
    ]);
  });

  it("normalizes a stale parent to null", () => {
    assert.deepEqual(toLayoutItems([t(1, 99)]), [{ id: 1, parent_id: null }]);
  });

  it("is empty for no tags", () => {
    assert.deepEqual(toLayoutItems([]), []);
  });
});
