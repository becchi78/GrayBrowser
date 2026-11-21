// index.html の同期スクリプトは描画前に走るため src/lib/appearance.ts を import
// できず、値の一覧とキーを重複して持つ。食い違うと「写しを読んでいるのに
// 反映されない」ことが黙って起きるので、ここで一致を保証する。
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { expect, test } from "vitest";
import { MODES, MODE_STORAGE_KEY, THEMES, THEME_STORAGE_KEY } from "./lib/appearance";

const html = readFileSync(resolve("index.html"), "utf8");

function listIn(name: string): string[] {
  const m = new RegExp(`var ${name} = \\[([^\\]]*)\\]`).exec(html);
  expect(m, `${name} の宣言が見つからない`).not.toBeNull();
  return [...(m as RegExpExecArray)[1].matchAll(/"([^"]+)"/g)].map((x) => x[1]);
}

test("同期スクリプトのテーマ・外観の一覧は appearance.ts と同じ", () => {
  expect(listIn("themes")).toEqual([...THEMES]);
  expect(listIn("modes")).toEqual([...MODES]);
});

test("同期スクリプトは appearance.ts と同じ localStorage のキーを読む", () => {
  expect(html).toContain(`getItem("${THEME_STORAGE_KEY}")`);
  expect(html).toContain(`getItem("${MODE_STORAGE_KEY}")`);
});

test("<html> の既定の属性は既定のテーマ・外観", () => {
  expect(html).toMatch(/<html[^>]*data-theme="console"/);
  expect(html).toMatch(/<html[^>]*data-mode="system"/);
});
