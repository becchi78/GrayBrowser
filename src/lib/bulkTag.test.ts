import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { filterTagCandidates } from "./bulkTag.ts";

describe("filterTagCandidates", () => {
  const tags = [
    { id: 1, name: "Action" },
    { id: 2, name: "drama" },
    { id: 3, name: "reaction" },
  ];

  it("returns every tag for an empty or whitespace-only input", () => {
    assert.equal(filterTagCandidates(tags, ""), tags);
    assert.equal(filterTagCandidates(tags, "   "), tags);
  });

  it("matches substrings case-insensitively", () => {
    assert.deepEqual(
      filterTagCandidates(tags, "ACTION").map((t) => t.id),
      [1, 3],
    );
  });

  it("returns an empty list when nothing matches", () => {
    assert.deepEqual(filterTagCandidates(tags, "zzz"), []);
  });

  it("returns an empty list for an empty tag list", () => {
    assert.deepEqual(filterTagCandidates([], "a"), []);
  });
});
