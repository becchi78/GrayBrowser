import { afterEach, expect, test } from "vitest";
import { readLengthToken } from "./lib/cssTokens";

afterEach(() => {
  document.documentElement.style.removeProperty("--probe-test");
});

test("解決できない・未定義の変数はフォールバックを返す", () => {
  expect(readLengthToken("--probe-test-undefined", 226)).toBe(226);
});

test("プローブ用の要素を残さない", () => {
  const before = document.body.childElementCount;
  readLengthToken("--probe-test-undefined", 1);
  expect(document.body.childElementCount).toBe(before);
});
