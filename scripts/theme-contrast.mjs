// src/themes.css の3テーマ × ライト/ダークで、主要な前景/背景の組が WCAG 2.x の
// AA を満たすか検査する。CSS を実際に評価する代わりに、themes.css が使う
// 記法(16進・rgb()/rgba()・var()・light-dark()・color-mix(in srgb))だけを
// 解釈する小さな評価器を持つ。未対応の記法に当たったら黙って通さず例外にする。
//
// 使い方: node scripts/theme-contrast.mjs
// 基準: 文字は 4.5:1、枠・アイコンなど文字以外の表示は 3:1。
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

export const THEMES = ["console", "fluent", "cinema"];
export const MODES = ["light", "dark"];

// ---- CSS の読み取り ----

/** `selector { decl; decl }` の並びを順に返す(ネストは無い前提)。 */
export function parseRules(css) {
  const stripped = css.replace(/\/\*[\s\S]*?\*\//g, "");
  const rules = [];
  const re = /([^{}]+)\{([^{}]*)\}/g;
  let m;
  while ((m = re.exec(stripped)) !== null) {
    const selectors = m[1].split(",").map((s) => s.trim());
    const decls = new Map();
    for (const part of splitTopLevel(m[2], ";")) {
      const idx = part.indexOf(":");
      if (idx === -1) continue;
      const name = part.slice(0, idx).trim();
      if (name.startsWith("--")) decls.set(name, part.slice(idx + 1).trim());
    }
    rules.push({ selectors, decls });
  }
  return rules;
}

/** 括弧の外にある `sep` で分割する。 */
function splitTopLevel(text, sep) {
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
 * テーマのトークン表。素の `:root` の宣言を先に、`:root[data-theme="X"]` を
 * 後に重ねる(CSS の詳細度の順と同じ)。`:root, :root[data-theme="console"]`
 * のような複合は、セレクタごとに別の宣言として扱う。
 */
export function tokensFor(rules, theme) {
  const tokens = new Map();
  const plain = (s) => s === ":root";
  const themed = (s) => s === `:root[data-theme="${theme}"]`;
  for (const pass of [plain, themed]) {
    for (const { selectors, decls } of rules) {
      if (!selectors.some(pass)) continue;
      for (const [k, v] of decls) tokens.set(k, v);
    }
  }
  return tokens;
}

// ---- 色の評価 ----

const clamp = (n, lo, hi) => Math.min(hi, Math.max(lo, n));

function parseHex(s) {
  let h = s.slice(1);
  if (![3, 4, 6, 8].includes(h.length)) throw new Error(`不正な16進色: ${s}`);
  if (h.length <= 4) h = [...h].map((c) => c + c).join("");
  const n = (i) => parseInt(h.slice(i, i + 2), 16);
  return { r: n(0), g: n(2), b: n(4), a: h.length === 8 ? n(6) / 255 : 1 };
}

/** `name` を { r, g, b, a } に評価する。 */
export function evalColor(expr, tokens, mode, seen = []) {
  const e = expr.trim();
  const fn = /^([a-z-]+)\(([\s\S]*)\)$/.exec(e);
  if (e.startsWith("#")) return parseHex(e);
  if (e === "transparent") return { r: 0, g: 0, b: 0, a: 0 };
  if (!fn) throw new Error(`解釈できない色: ${e}`);
  const [, name, inner] = fn;
  const args = splitTopLevel(inner, ",");
  const sub = (x) => evalColor(x, tokens, mode, seen);
  switch (name) {
    case "var": {
      const token = args[0];
      if (seen.includes(token)) throw new Error(`変数の循環: ${[...seen, token].join(" -> ")}`);
      const value = tokens.get(token);
      if (value === undefined) throw new Error(`未定義の変数: ${token}`);
      return evalColor(value, tokens, mode, [...seen, token]);
    }
    case "light-dark":
      return sub(mode === "light" ? args[0] : args[1]);
    case "rgb":
    case "rgba": {
      const [r, g, b, a = "1"] = args;
      return { r: Number(r), g: Number(g), b: Number(b), a: Number(a) };
    }
    case "color-mix": {
      if (args[0].replace(/\s+/g, " ") !== "in srgb") {
        throw new Error(`color-mix は in srgb のみ対応: ${e}`);
      }
      const parts = args.slice(1, 3).map((a) => {
        const pm = /^([\s\S]*?)\s+(\d+(?:\.\d+)?)%$/.exec(a);
        return pm ? { color: sub(pm[1]), pct: Number(pm[2]) } : { color: sub(a), pct: null };
      });
      let [p1, p2] = [parts[0].pct, parts[1].pct];
      if (p1 === null && p2 === null) [p1, p2] = [50, 50];
      else if (p1 === null) p1 = 100 - p2;
      else if (p2 === null) p2 = 100 - p1;
      const sum = p1 + p2;
      const w1 = p1 / sum;
      const w2 = p2 / sum;
      const [c1, c2] = [parts[0].color, parts[1].color];
      // 不透明度を持つ色は、仕様どおり事前乗算して混ぜる。
      const a = c1.a * w1 + c2.a * w2;
      const mixCh = (ch) => (a === 0 ? 0 : (c1[ch] * c1.a * w1 + c2[ch] * c2.a * w2) / a);
      return { r: mixCh("r"), g: mixCh("g"), b: mixCh("b"), a };
    }
    default:
      throw new Error(`未対応の関数 ${name}(): ${e}`);
  }
}

/** 半透明の前景色を不透明な背景に重ねた色。 */
export function over(fg, bg) {
  const a = fg.a;
  return {
    r: fg.r * a + bg.r * (1 - a),
    g: fg.g * a + bg.g * (1 - a),
    b: fg.b * a + bg.b * (1 - a),
    a: 1,
  };
}

function luminance({ r, g, b }) {
  const lin = (v) => {
    const c = clamp(v, 0, 255) / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
}

export function contrastRatio(fg, bg) {
  const [l1, l2] = [luminance(fg), luminance(bg)].sort((x, y) => y - x);
  return (l1 + 0.05) / (l2 + 0.05);
}

// ---- 検査する組 ----

const TEXT = 4.5;
const UI = 3;

// [前景, 背景, 基準, 説明]。トークン名のほか、
// "pressed(--x)" は --pressed-tint を --x の上に重ねた色を表す。
const PAIRS = [];
const textOn = (fgs, bgs, min = TEXT) => {
  for (const fg of fgs) for (const bg of bgs) PAIRS.push([fg, bg, min]);
};
// opacity で文字を薄くする箇所は、色トークンだけを測ると取りこぼす(色の上に
// さらに opacity が掛かって背景に溶ける)。[前景, 背景, 基準, 不透明度] として、
// 前景に不透明度を掛けた見た目で測る。
const textWithOpacity = (opacities, fgs, bgs, min = TEXT) => {
  for (const o of opacities) {
    for (const fg of fgs) for (const bg of bgs) PAIRS.push([fg, bg, min, o]);
  }
};

const SURFACES = ["--page-bg", "--surface", "--surface-2"];

textOn(["--text", "--text-muted", "--text-subtle"], SURFACES);
// 行の上に出るのは本文と補助(muted)の文字。text-subtle は行の上では使わない
// (仕様の text-subtle は、ライトの Fluent で hover/選択行の上だと 4.4:1 になる)。
textOn(["--text", "--text-muted"], ["--hover-bg", "--row-selected-bg"]);
textOn(["--text", "--text-muted"], ["--row-multi-selected-bg"]);
textOn(["--chip-text", "--control-text"], ["--chip-bg"]);
textOn(
  ["--control-text"],
  ["--control-bg", "--control-hover-bg", "--control-active-bg", "pressed(--control-bg)"],
);
textOn(["--text"], ["--chip-bg", "--thumbnail-bg", "--update-notice-bg", "--notes-bg"]);
// 左ペインの選択項目・押下中のバッジ・主要ボタンは accent の塗り。
textOn(["--accent-fg"], ["--accent"]);
textOn(["--on-selected"], ["--selected-bg"]);
textOn(["--on-danger"], ["--danger-bg"]);
// リンク・アクセント色の文字・件数など。
textOn(["--accent", "--link"], SURFACES);
textOn(["--accent"], ["--row-selected-bg", "--hover-bg"]);
// 状態の文字色は、面と、それを薄く混ぜた背景の両方で読める必要がある。
textOn(["--error-text"], [...SURFACES, "--result-failed-bg", "--failure-badge-bg"]);
textOn(["--warn-text"], [...SURFACES, "--warn-bg", "--diff-bg", "--offline-bg", "--notice-bg"]);
textOn(["--success-text", "--success-notice-text"], [...SURFACES, "--success-bg"]);
textOn(["--notice-text"], ["--notice-bg"]);
textOn(["--offline-text"], ["--offline-bg"]);
textOn(["--diff-text"], ["--diff-bg"]);
textOn(["--thumbnail-text"], ["--thumbnail-bg"]);
textOn(["--thumbnail-failed-text"], ["--thumbnail-failed-bg"]);
textOn(["--thumbnail-offline-text"], ["--thumbnail-offline-bg"]);
textOn(["--status-progress-text", "--failures-notice-text"], SURFACES);
textOn(["--log-error", "--log-warn", "--log-info", "--log-debug", "--log-trace"], SURFACES);
// 文字以外: ★・状態色のアイコン・フォーカスリング。仕様の枠線色(--border-strong)は
// 装飾の区切りで、仕様の値のまま使うため対象にしない。
textOn(["--star", "--star-color", "--danger", "--warn", "--success"], SURFACES, UI);
textOn(["--star", "--star-color"], ["--row-selected-bg"], UI);
textOn(["--accent"], [...SURFACES, "--row-selected-bg", "--hover-bg"], UI);

// App.css で文字に opacity を掛けている箇所(0.7〜0.85 と --opacity-muted の 0.75)。
// 無効状態(:disabled)・進行中の手順の薄表示・装飾は AA の対象外なので含めない。
// 本文色のまま薄くするもの: ラベル(dt)・補助の説明・パスなど。
const DIM_OPACITIES = [0.7, 0.75, 0.8, 0.85];
textWithOpacity(DIM_OPACITIES, ["--text"], [...SURFACES, "--hover-bg", "--row-selected-bg"]);
textWithOpacity(DIM_OPACITIES, ["--text"], ["--row-multi-selected-bg"]);
// 検索ボックスのクリアボタン(アイコン)は 0.6 で、文字ではないので 3:1。
textWithOpacity([0.6], ["--text"], SURFACES, UI);
// シネマの非選択行は、情報パネル全体の文字が --text-muted になる。その中の
// ラベル(dt)に opacity を重ねると AA を割るので、App.css 側で dt の opacity を
// 外している。ここでは、その状態(--text-muted を不透明で使う)が AA を満たすことを測る。
textOn(["--text-muted"], ["--page-bg", "--surface", "--surface-2"]);

/** 前景/背景の表現("--x" か "pressed(--x)")を不透明な色にする。 */
function resolveSide(side, tokens, mode) {
  const pressed = /^pressed\((--[\w-]+)\)$/.exec(side);
  if (pressed) {
    const base = evalColor(`var(${pressed[1]})`, tokens, mode);
    return over(evalColor("var(--pressed-tint)", tokens, mode), base);
  }
  return evalColor(`var(${side})`, tokens, mode);
}

/** 全テーマ・全外観の検査結果。{ theme, mode, fg, bg, ratio, min, ok } の配列。 */
export function checkAll(css) {
  const rules = parseRules(css);
  const results = [];
  for (const theme of THEMES) {
    const tokens = tokensFor(rules, theme);
    for (const mode of MODES) {
      for (const [fg, bg, min, opacity = 1] of PAIRS) {
        const bgColor = resolveSide(bg, tokens, mode);
        // 前景が半透明なら、背景に重ねた見た目で測る。opacity は前景の
        // アルファに掛かる(要素全体の不透明度は文字色の不透明度と同じ合成になる)。
        const solid = resolveSide(fg, tokens, mode);
        const fgColor = over({ ...solid, a: solid.a * opacity }, bgColor);
        const ratio = contrastRatio(fgColor, bgColor);
        results.push({ theme, mode, fg, bg, opacity, ratio, min, ok: ratio >= min });
      }
    }
  }
  return results;
}

function main() {
  const here = dirname(fileURLToPath(import.meta.url));
  const css = readFileSync(join(here, "..", "src", "themes.css"), "utf8");
  const results = checkAll(css);
  const failures = results.filter((r) => !r.ok);
  for (const r of failures) {
    console.error(
      `NG  ${r.theme}/${r.mode}  ${r.fg}${r.opacity === 1 ? "" : ` x${r.opacity}`} on ${r.bg}  ${r.ratio.toFixed(2)} < ${r.min}`,
    );
  }
  console.log(
    `${results.length} pairs (${THEMES.length} themes x ${MODES.length} modes), ` +
      `${failures.length} failing, min ratio ${Math.min(...results.map((r) => r.ratio)).toFixed(2)}`,
  );
  process.exit(failures.length === 0 ? 0 : 1);
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) main();
