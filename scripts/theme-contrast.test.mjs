// theme-contrast のテスト。package.json の test:unit 参照。
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { checkAll, contrastRatio, evalColor, over, parseRules, tokensFor } from "./theme-contrast.mjs";

const themesCss = readFileSync(new URL("../src/themes.css", import.meta.url), "utf8");

const tokens = (css, theme = "console") => tokensFor(parseRules(css), theme);

test("白と黒は 21:1、同色は 1:1", () => {
  const black = { r: 0, g: 0, b: 0, a: 1 };
  const white = { r: 255, g: 255, b: 255, a: 1 };
  assert.ok(Math.abs(contrastRatio(white, black) - 21) < 1e-9);
  assert.equal(contrastRatio(white, white), 1);
});

test("light-dark は外観で選び、var は参照先を評価する", () => {
  const t = tokens(":root { --a: light-dark(#ffffff, #000000); --b: var(--a); }");
  assert.deepEqual(evalColor("var(--b)", t, "light"), { r: 255, g: 255, b: 255, a: 1 });
  assert.deepEqual(evalColor("var(--b)", t, "dark"), { r: 0, g: 0, b: 0, a: 1 });
});

test("color-mix は割合で混ぜ、片方だけ割合があれば残りを補う", () => {
  const t = tokens(":root { --w: #ffffff; --k: #000000; }");
  const mid = evalColor("color-mix(in srgb, var(--w), var(--k))", t, "light");
  assert.equal(Math.round(mid.r), 128);
  const quarter = evalColor("color-mix(in srgb, var(--w) 25%, var(--k))", t, "light");
  assert.equal(Math.round(quarter.r), 64);
});

test("半透明は背景に重ねて測る", () => {
  const half = over({ r: 255, g: 255, b: 255, a: 0.5 }, { r: 0, g: 0, b: 0, a: 1 });
  assert.equal(Math.round(half.r), 128);
});

test("素の :root の後にテーマ別の宣言が重なる(複合セレクタはセレクタごと)", () => {
  const css = `
    :root, :root[data-theme="console"] { --x: #111111; }
    :root[data-theme="fluent"] { --x: #222222; }
    :root { --y: var(--x); }`;
  assert.equal(tokens(css, "console").get("--x"), "#111111");
  assert.equal(tokens(css, "fluent").get("--x"), "#222222");
  assert.equal(tokens(css, "cinema").get("--x"), "#111111");
  assert.equal(tokens(css, "fluent").get("--y"), "var(--x)");
});

test("未定義の変数・未対応の記法・循環は例外にする(黙って通さない)", () => {
  const t = tokens(":root { --a: var(--b); --b: var(--a); --c: hsl(0 0% 0%); }");
  assert.throws(() => evalColor("var(--missing)", t, "light"), /未定義/);
  assert.throws(() => evalColor("var(--a)", t, "light"), /循環/);
  assert.throws(() => evalColor("var(--c)", t, "light"), /未対応/);
});

test("src/themes.css は全テーマ・全外観で AA を満たす", () => {
  const failures = checkAll(themesCss).filter((r) => !r.ok);
  assert.deepEqual(
    failures.map((r) => `${r.theme}/${r.mode} ${r.fg} on ${r.bg} ${r.ratio.toFixed(2)}`),
    [],
  );
});

test("コントラストが足りない色になれば検査は失敗する", () => {
  const bad = themesCss.replace(
    /--text-muted: light-dark\(#42506a, #9aa8bf\);/,
    "--text-muted: light-dark(#d6dbe4, #0d1526);",
  );
  assert.notEqual(bad, themesCss);
  assert.ok(checkAll(bad).some((r) => !r.ok));
});
