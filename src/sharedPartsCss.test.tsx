// 件数ヘッダー・左ペインの件数ピル・行頭のチェックボックスは、全テーマ共通の部品。
// どこかのテーマの指定で隠してしまうと、jsdom では CSS が効かず気付けないので、
// CSS のテキストで見張る。
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { expect, test } from "vitest";

// コメントが次の規則のセレクタに混ざらないよう、先に取り除く。
const css = readFileSync(resolve("src/App.css"), "utf8").replace(
  /\/\*[\s\S]*?\*\//g,
  "",
);
const themes = readFileSync(resolve("src/themes.css"), "utf8");

// 宣言ブロックごとに [セレクタ, 本体] へ分ける。@media の中の規則も、`{` と `}` の
// 間にある単純な規則として拾える(セレクタの前に @media の見出しが付くだけ)。
function rulesFor(className: string): { selector: string; body: string }[] {
  const all = [...css.matchAll(/([^{}]+)\{([^}]*)\}/g)].map((m) => ({
    selector: m[1].trim(),
    body: m[2],
  }));
  // 子孫・複合セレクタも含め、その部品を指すものすべて。なお、情報パネルを隠す
  // `.video-list--info-collapsed .video-row-info` は、これらの部品を含まないので対象外。
  const re = new RegExp(String.raw`\.${className}(?![\w-])`);
  return all.filter((r) => re.test(r.selector));
}

test.each(["list-header", "count-pill", "video-row-check"])(
  ".%s を含むどの規則も、非表示にしない。既定の規則は表示する",
  (name) => {
    const rules = rulesFor(name);
    expect(rules.length).toBeGreaterThan(0);
    for (const { selector, body } of rules) {
      expect(body, selector).not.toMatch(/display:\s*none/);
      expect(body, selector).not.toMatch(/visibility:\s*hidden/);
      expect(selector, selector).not.toContain("[hidden]");
    }
    expect(
      rules.some(
        (r) =>
          r.selector === `.${name}` && /display:\s*(flex|block)/.test(r.body),
      ),
    ).toBe(true);
  },
);

test("チェックボックスの列幅は、全テーマが 0 より大きい値を持つ", () => {
  const widths = [...themes.matchAll(/--check-w:\s*(\d+)px/g)].map((m) =>
    Number(m[1]),
  );
  expect(widths).toHaveLength(3);
  for (const w of widths) expect(w).toBeGreaterThan(0);
});

test("絞り込み中バーには件数用のスタイルが残っていない", () => {
  expect(css).not.toContain("filter-bar-count");
});
