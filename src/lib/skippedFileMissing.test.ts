import assert from "node:assert/strict";
import { test } from "node:test";
import { missingRegistrationNotice } from "./skippedFileMissing.ts";

test("no notice when the .wb data was carried over", () => {
  assert.equal(missingRegistrationNotice(true), null);
});

test("without .wb data the notice says to re-import before registering", () => {
  const notice = missingRegistrationNotice(false);
  assert.ok(notice !== null);
  assert.ok(notice.includes("登録の前に"));
});
