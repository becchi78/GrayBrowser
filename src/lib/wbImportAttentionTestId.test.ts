import assert from "node:assert/strict";
import { test } from "node:test";
import { attentionTestId } from "./wbImportAttentionTestId.ts";

test("attentionTestId: 専用の data-testid を持つ項目", () => {
  assert.equal(attentionTestId("unmatched"), "import-result-needs-review");
  assert.equal(attentionTestId("unmatched-empty-hash"), "import-result-empty-hash");
  assert.equal(attentionTestId("invalid-name"), "import-result-skipped-invalid-name");
  assert.equal(attentionTestId("failed-rows"), "import-result-failed-rows");
  assert.equal(attentionTestId("skipped-roots"), "import-result-skipped-roots");
  assert.equal(
    attentionTestId("volume-label-unresolved"),
    "import-result-volume-label-unresolved",
  );
});

test("attentionTestId: data-testid を持たない項目は undefined", () => {
  assert.equal(attentionTestId("failed"), undefined);
  assert.equal(attentionTestId("tags"), undefined);
});
