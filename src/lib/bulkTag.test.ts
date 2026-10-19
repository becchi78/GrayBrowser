import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { bulkTagResultLines, filterTagCandidates } from "./bulkTag.ts";

describe("bulkTagResultLines", () => {
  it("reports only the newly tagged count when nothing else happened", () => {
    assert.deepEqual(bulkTagResultLines("action", { newlyTagged: 3, alreadyTagged: 0, missing: 0 }), [
      "タグ「action」を 3 件に付けました。",
    ]);
  });

  it("adds the already-tagged and missing counts when present", () => {
    assert.deepEqual(bulkTagResultLines("a", { newlyTagged: 2, alreadyTagged: 1, missing: 4 }), [
      "タグ「a」を 2 件に付けました。",
      "1 件は既に付いていました。",
      "4 件は見つかりませんでした(削除または移動された可能性があります)。",
    ]);
  });

  it("omits the newly tagged line when every video already had the tag", () => {
    assert.deepEqual(bulkTagResultLines("a", { newlyTagged: 0, alreadyTagged: 5, missing: 0 }), [
      "5 件は既に付いていました。",
    ]);
  });

  it("falls back to a message when there was nothing to tag", () => {
    assert.deepEqual(bulkTagResultLines("a", { newlyTagged: 0, alreadyTagged: 0, missing: 0 }), [
      "対象の動画がありませんでした。",
    ]);
  });
});

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
