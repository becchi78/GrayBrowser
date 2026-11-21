// テーマの CSS とマニフェストを読む共通の部品。コントラスト検査と、構造の検査
// (scripts/theme-structure.test.mjs)が使う。CSS は実際には評価せず、
// 「セレクタ { 宣言 }」の並びとして読むだけの簡易パーサで扱う。
import { readFileSync, readdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
export const SRC_DIR = join(here, "..", "src");
export const THEMES_DIR = join(SRC_DIR, "themes");

/** 改行コードの違い(チェックアウトの設定)で結果が変わらないよう LF に揃えて読む。 */
export function readText(path) {
  return readFileSync(path, "utf8").replace(/\r\n/g, "\n");
}

export function readManifest() {
  return JSON.parse(readFileSync(join(THEMES_DIR, "manifest.json"), "utf8"));
}

/** 共通の土台(_base.css)の CSS。 */
export function readBaseCss() {
  return readText(join(THEMES_DIR, "_base.css"));
}

export function readThemeCss(theme) {
  return readText(join(THEMES_DIR, `${theme}.css`));
}

/** src/themes.css の @import と同じ順(_base → マニフェストの順の各テーマ)に連結した CSS。 */
export function readAllThemesCss(manifest = readManifest()) {
  return [readBaseCss(), ...manifest.themes.map(readThemeCss)].join("\n");
}

/** src/themes/ 直下の、_base 以外の CSS ファイルのテーマ名(拡張子なし、昇順)。 */
export function themeFileNames() {
  return readdirSync(THEMES_DIR)
    .filter((f) => f.endsWith(".css") && !f.startsWith("_"))
    .map((f) => f.slice(0, -".css".length))
    .sort();
}

export function stripComments(css) {
  return css.replace(/\/\*[\s\S]*?\*\//g, "");
}

/** 括弧の外にある `sep` で分割する。 */
export function splitTopLevel(text, sep) {
  const parts = [];
  let depth = 0;
  let start = 0;
  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if (c === "(") depth++;
    else if (c === ")") depth--;
    else if (c === sep && depth === 0) {
      parts.push(text.slice(start, i));
      start = i + 1;
    }
  }
  parts.push(text.slice(start));
  return parts.map((p) => p.trim()).filter((p) => p !== "");
}

/**
 * `selector { decl; decl }` の並びを、ソース上の順に返す。@media の中の規則も、
 * 見出しをセレクタの前に付けた単純な規則として拾う。セレクタはカンマで分け、
 * 空白は1つに詰める。宣言は同じプロパティが複数あれば後のものが残る。
 */
export function parseDeclarations(css) {
  const rules = [];
  const re = /([^{}]+)\{([^{}]*)\}/g;
  const stripped = stripComments(css);
  let m;
  while ((m = re.exec(stripped)) !== null) {
    const selectors = splitTopLevel(m[1], ",").map((s) => s.replace(/\s+/g, " "));
    const decls = new Map();
    for (const part of splitTopLevel(m[2], ";")) {
      const idx = part.indexOf(":");
      if (idx === -1) continue;
      decls.set(part.slice(0, idx).trim(), part.slice(idx + 1).trim().replace(/\s+/g, " "));
    }
    rules.push({ selectors, decls });
  }
  return rules;
}

/**
 * テーマのファイルが「自分の定義」として持てるセレクタ。`:root[data-theme="<theme>"]`
 * に加え、既定テーマのファイルだけは素の `:root`(未設定・未知の data-theme での
 * 既定を兼ねる `:root, :root[data-theme="..."]`)を持てる。他のテーマのファイルに
 * 素の `:root` を書くと全テーマに効いてしまうので、自分の定義とは認めない。
 */
export function ownSelectors(theme, manifest = readManifest()) {
  const own = new Set([`:root[data-theme="${theme}"]`]);
  if (theme === manifest.defaultTheme) own.add(":root");
  return own;
}

/** テーマのファイルがそのテーマとして定義しているプロパティ名。 */
export function definedProperties(css, theme, manifest = readManifest()) {
  const own = ownSelectors(theme, manifest);
  const defined = new Set();
  for (const { selectors, decls } of parseDeclarations(css)) {
    if (!selectors.some((s) => own.has(s))) continue;
    for (const name of decls.keys()) defined.add(name);
  }
  return defined;
}

/** 必須のトークン・プロパティのうち、テーマの CSS に無いもの。 */
export function missingRequired(css, theme, manifest = readManifest()) {
  const defined = definedProperties(css, theme, manifest);
  return [...manifest.requiredProperties, ...manifest.requiredTokens].filter(
    (name) => !defined.has(name),
  );
}

/** テーマのファイルにある、自分の定義(ownSelectors)以外のセレクタ。 */
export function selectorOverrides(css, theme, manifest = readManifest()) {
  const own = ownSelectors(theme, manifest);
  return parseDeclarations(css)
    .flatMap((r) => r.selectors)
    .filter((s) => !own.has(s));
}
