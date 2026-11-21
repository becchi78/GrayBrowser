// 「テーマの差をトークンで表す」前後で、見た目が変わっていないことの検査。
// package.json の test:unit 参照。
//
// 旧い実装は、テーマごとに `:root[data-theme=...] <部品>` というセレクタで部品の
// プロパティを上書きしていた。その最終的な宣言を、下の EXPECTED に値の表として固定し、
// 新しい CSS(テーマのトークンの値 + 部品の CSS)から同じ値が得られることを確かめる。
//
// 対象は、セレクタ方式だった時点(v0.17.0)に存在した3テーマだけ。目的は「整理で
// 見た目が変わっていないこと」と、その後の部品 CSS の変更でこの3テーマが意図せず
// 変わらないことの固定で、新しいテーマの値を縛るものではない。したがって新しい
// テーマを足しても、ここに行を足す必要は無い(足さなくても落ちない)。
//
// CSS 全体を解決するエンジンは使えない(jsdom は light-dark() やカスケードを正しく
// 扱わない)ので、次の簡易な解決にとどめる。
// - 状態ごとに、その状態に当てはまる部品のセレクタを、カスケードの順に自分で並べる。
// - 並べたセレクタに一致する規則の宣言を、ソースの順に重ねる。
// - 新しく導入したトークンの var() だけを、テーマの値に置き換える。それ以外の
//   var(--surface) などは、旧い宣言と同じ書き方のまま比べる。
// - 省略記法(border-width・border-radius・padding)は 上 右 下 左 の4値に展開する。
//   幅がすべて 0 のときは、枠の色は見えないので比べない。
// 色などの最終的な値(light-dark() の中身)は theme-contrast の検査と、各テーマの
// トークンの値そのもの(トークンは仕様の値のまま)で守られている。
import assert from "node:assert/strict";
import { test } from "node:test";
import { tokensFor, parseRules } from "./theme-contrast.mjs";
import { SRC_DIR, parseDeclarations, readAllThemesCss, readManifest, readText } from "./theme-css.mjs";
import { join } from "node:path";

const IDLE =
  ".video-row:not(.video-row--selected, .video-row--multi-selected, :hover, :focus-within)";
const IDLE_INFO = `${IDLE} .video-row-info`;
const IDLE_DT = `${IDLE} .video-row-properties dt`;

/** 今回トークン化した値。部品の CSS からこの名前の var() だけをテーマの値に置き換える。 */
const NEW_TOKENS = new Set([
  "--layer-bg",
  "--layer-border-width",
  "--layer-radius",
  "--layer-overflow",
  "--row-bg",
  "--row-border-width",
  "--row-radius",
  "--row-selected-border-color",
  "--row-selected-shadow",
  "--row-multi-selected-shadow",
  "--info-padding",
  "--info-border-width",
  "--info-radius",
  "--info-bg",
  "--info-idle-color",
  "--info-idle-dt-opacity",
  "--list-header-letter-spacing",
  "--info-w",
  "--thumb-gap",
  "--thumb-radius",
  "--nav-w",
  "--nav-item-h",
  "--shadow-control",
  "--shadow-popover",
  "--shadow-popover-up",
  "--shadow-dialog",
  "--font-mono",
]);

/** 宣言が無いときの既定(CSS の初期値)。 */
const DEFAULTS = {
  "background-color": "transparent",
  "border-width": "0 0 0 0",
  "border-radius": "0 0 0 0",
  padding: "0 0 0 0",
  overflow: "visible",
  "box-shadow": "none",
  color: "currentcolor",
  "letter-spacing": "normal",
};

const FOUR = new Set(["border-width", "border-radius", "padding"]);

/** 空白で区切る(括弧の中の空白は区切らない)。 */
function splitWhitespace(text) {
  const parts = [];
  let depth = 0;
  let cur = "";
  for (const c of text.trim()) {
    if (c === "(") depth++;
    if (c === ")") depth--;
    if (/\s/.test(c) && depth === 0) {
      if (cur) parts.push(cur);
      cur = "";
    } else cur += c;
  }
  if (cur) parts.push(cur);
  return parts;
}

function expandFour(value) {
  const p = splitWhitespace(value);
  const [t, r = t, b = t, l = r] = p;
  return [t, r, b, l ?? t].map((v) => (v === "0px" ? "0" : v)).join(" ");
}

/** 新しいトークンの var() をテーマの値に置き換える(値が別のトークンを含んでもよい)。 */
function substitute(value, tokens) {
  let out = value;
  for (let i = 0; i < 5; i++) {
    const next = out.replace(/var\((--[\w-]+)\)/g, (whole, name) =>
      NEW_TOKENS.has(name) ? (tokens.get(name) ?? whole) : whole,
    );
    if (next === out) return out;
    out = next;
  }
  throw new Error(`トークンの置き換えが収まらない: ${value}`);
}

/** セレクタの並び(カスケードの順)に当てはまる規則を重ねた、プロパティの最終値。 */
function resolve(appRules, tokens, selectors, props) {
  const decls = new Map();
  for (const sel of selectors) {
    const matched = appRules.filter((r) => r.selectors.includes(sel));
    assert.ok(matched.length > 0, `App.css に ${sel} の規則が無い`);
    for (const r of matched) for (const [k, v] of r.decls) decls.set(k, v);
  }
  const result = {};
  for (const prop of props) {
    let v = decls.get(prop) ?? DEFAULTS[prop];
    if (v === undefined) continue;
    v = substitute(v, tokens);
    if (FOUR.has(prop)) v = expandFour(v);
    result[prop] = v;
  }
  // 幅がすべて 0 の枠は描かれないので、色は比べない。
  if (result["border-width"] === "0 0 0 0") delete result["border-color"];
  return result;
}

const all4 = (v) => `${v} ${v} ${v} ${v}`;
const R = "var(--row-border)";
const NONE = "0 0 0 0";
const MULTI = "inset 4px 0 0 var(--row-multi-selected-accent)";
const RING = "inset 0 0 0 1px var(--accent)";

// [状態の名前, カスケード順のセレクタ, 比べるプロパティ]
const SCENARIOS = [
  [
    "list-layer",
    [".list-layer"],
    ["background-color", "border-width", "border-color", "border-radius", "overflow"],
  ],
  ["list-header-title", [".list-header-title"], ["letter-spacing"]],
  [
    "row",
    [".video-row"],
    ["border-width", "border-color", "border-radius", "background-color", "box-shadow"],
  ],
  [
    "row-selected",
    [".video-row", ".video-row--selected"],
    ["border-width", "border-color", "border-radius", "background-color", "box-shadow"],
  ],
  [
    "row-selected-hover",
    [".video-row", ".video-row--selected", ".video-row--selected:hover"],
    ["border-color", "background-color", "box-shadow"],
  ],
  [
    "row-multi-selected-only",
    [".video-row", ".video-row--multi-selected"],
    ["border-color", "background-color", "box-shadow"],
  ],
  [
    "row-selected-and-multi",
    [".video-row", ".video-row--selected", ".video-row--multi-selected"],
    ["border-color", "background-color", "box-shadow"],
  ],
  [
    "row-info",
    [".video-row-info"],
    ["padding", "border-width", "border-color", "border-radius", "background-color", "color"],
  ],
  ["row-info-idle", [".video-row-info", IDLE_INFO], ["color"]],
  ["row-label", [".video-row-properties dt"], ["opacity"]],
  ["row-label-idle", [".video-row-properties dt", IDLE_DT], ["opacity"]],
  ["thumbnails-row", [".video-row-thumbnails"], ["gap"]],
  ["thumbnail-image", [".video-row-thumbnail-wrap img"], ["border-radius"]],
  ["thumbnail-placeholder", [".thumbnail-placeholder"], ["border-radius"]],
  ["nav-folder-item", [".folder-sidebar-item"], ["height"]],
  ["nav-rating-item", [".rating-bar-item"], ["height"]],
  ["info-column", [".video-row-info"], ["width", "flex"]],
];

// 旧い CSS(テーマ別セレクタ)の最終的な宣言。共通の既定は同じ値を、テーマごとに書く。
const rowBase = {
  console: { "border-width": "0 0 1px 0", "border-color": R, "border-radius": NONE, "background-color": "transparent" },
  fluent: {
    "border-width": all4("1px"),
    "border-color": R,
    "border-radius": all4("var(--radius-md)"),
    "background-color": "var(--surface-2)",
  },
};
rowBase.cinema = rowBase.console;

const infoPlain = {
  padding: NONE,
  "border-width": NONE,
  "border-radius": NONE,
  "background-color": "transparent",
  color: "currentcolor",
};

/** 3テーマで共通だった値(サムネイルの間隔・角丸、左ペインの項目の高さ、情報パネルの幅)。 */
const sharedLegacy = {
  "thumbnails-row": { gap: "0.4em" },
  "thumbnail-image": { "border-radius": all4("var(--radius-sm)") },
  "thumbnail-placeholder": { "border-radius": all4("var(--radius-sm)") },
  "nav-folder-item": { height: "auto" },
  "nav-rating-item": { height: "auto" },
  "info-column": { width: "300px", flex: "0 0 300px" },
};

export const EXPECTED = {
  console: {
    "list-layer": {
      "background-color": "var(--surface)",
      "border-width": all4("1px"),
      "border-color": R,
      "border-radius": all4("var(--radius-md)"),
      overflow: "hidden",
    },
    "list-header-title": { "letter-spacing": "0.08em" },
    row: { ...rowBase.console, "box-shadow": "none" },
    "row-selected": {
      ...rowBase.console,
      "background-color": "var(--row-selected-bg)",
      "border-color": R,
      "box-shadow": "none",
    },
    "row-selected-hover": { "border-color": R, "background-color": "var(--row-selected-bg)", "box-shadow": "none" },
    "row-multi-selected-only": { "border-color": R, "background-color": "var(--row-multi-selected-bg)", "box-shadow": MULTI },
    "row-selected-and-multi": { "border-color": R, "background-color": "var(--row-multi-selected-bg)", "box-shadow": MULTI },
    "row-info": {
      padding: all4("var(--space-2)"),
      "border-width": all4("1px"),
      "border-color": R,
      "border-radius": all4("var(--radius-md)"),
      "background-color": "var(--surface-2)",
      color: "currentcolor",
    },
    "row-info-idle": { color: "currentcolor" },
    "row-label": { opacity: "0.7" },
    "row-label-idle": { opacity: "0.7" },
  },
  fluent: {
    "list-layer": {
      "background-color": "var(--surface)",
      "border-width": "1px 0 0 1px",
      "border-color": R,
      "border-radius": "var(--radius-md) 0 0 0",
      overflow: "visible",
    },
    "list-header-title": { "letter-spacing": "normal" },
    row: { ...rowBase.fluent, "box-shadow": "none" },
    "row-selected": {
      ...rowBase.fluent,
      "background-color": "var(--row-selected-bg)",
      "border-color": "var(--accent)",
      "box-shadow": "none",
    },
    "row-selected-hover": { "border-color": "var(--accent)", "background-color": "var(--row-selected-bg)", "box-shadow": "none" },
    "row-multi-selected-only": { "border-color": R, "background-color": "var(--row-multi-selected-bg)", "box-shadow": MULTI },
    "row-selected-and-multi": { "border-color": "var(--accent)", "background-color": "var(--row-multi-selected-bg)", "box-shadow": MULTI },
    "row-info": infoPlain,
    "row-info-idle": { color: "currentcolor" },
    "row-label": { opacity: "0.7" },
    "row-label-idle": { opacity: "0.7" },
  },
  cinema: {
    "list-layer": {
      "background-color": "transparent",
      "border-width": NONE,
      "border-radius": NONE,
      overflow: "visible",
    },
    "list-header-title": { "letter-spacing": "normal" },
    row: { ...rowBase.cinema, "box-shadow": "none" },
    "row-selected": {
      ...rowBase.cinema,
      "background-color": "var(--row-selected-bg)",
      "border-color": R,
      "box-shadow": RING,
    },
    "row-selected-hover": { "border-color": R, "background-color": "var(--row-selected-bg)", "box-shadow": RING },
    "row-multi-selected-only": { "border-color": R, "background-color": "var(--row-multi-selected-bg)", "box-shadow": `${RING}, ${MULTI}` },
    "row-selected-and-multi": { "border-color": R, "background-color": "var(--row-multi-selected-bg)", "box-shadow": `${RING}, ${MULTI}` },
    "row-info": infoPlain,
    "row-info-idle": { color: "var(--text-muted)" },
    "row-label": { opacity: "0.7" },
    "row-label-idle": { opacity: "1" },
  },
};

for (const theme of Object.keys(EXPECTED)) Object.assign(EXPECTED[theme], sharedLegacy);

/** 影・等幅フォント・左ペインの初期幅は、旧い実装では全テーマ共通の固定値だった。 */
const LEGACY_FIXED_TOKENS = {
  "--shadow-control": "0 1px 2px rgba(0, 0, 0, 0.12)",
  "--shadow-popover-up": "0 -4px 12px rgba(0, 0, 0, 0.2)",
  "--shadow-popover": "0 4px 12px rgba(0, 0, 0, 0.2)",
  "--shadow-dialog": "0 8px 24px rgba(0, 0, 0, 0.3)",
  "--font-mono": 'Consolas, "Cascadia Mono", monospace',
  "--nav-w": "200px",
  "--info-w": "300px",
  "--thumb-gap": "0.4em",
  "--thumb-radius": "var(--radius-sm)",
  "--nav-item-h": "auto",
};

const manifest = readManifest();
const LEGACY_THEMES = Object.keys(EXPECTED);
const appCss = readText(join(SRC_DIR, "App.css"));

/** 全テーマ × 状態の、新しい CSS から解決した値。 */
export function resolveAll(app, themesCss) {
  const appRules = parseDeclarations(app);
  const themeRules = parseRules(themesCss);
  const out = {};
  for (const theme of LEGACY_THEMES) {
    const tokens = tokensFor(themeRules, theme);
    out[theme] = {};
    for (const [name, selectors, props] of SCENARIOS) {
      out[theme][name] = resolve(appRules, tokens, selectors, props);
    }
  }
  return out;
}

test("テーマ別セレクタをトークンに置き換えても、全テーマ・全状態の宣言は旧い実装と同じ", () => {
  // 対象のテーマは、マニフェストから消えていない(消すなら、この表も意図して直す)。
  for (const theme of LEGACY_THEMES) assert.ok(manifest.themes.includes(theme), theme);
  const actual = resolveAll(appCss, readAllThemesCss());
  for (const theme of LEGACY_THEMES) {
    for (const [name] of SCENARIOS) {
      assert.deepEqual(actual[theme][name], EXPECTED[theme][name], `${theme} / ${name}`);
    }
  }
});

test("テーマのトークンの値を変えると、同値の検査は落ちる", () => {
  const css = readAllThemesCss();
  const mutate = (from, to) => {
    const next = css.replace(from, to);
    assert.notEqual(next, css, `${from} を置き換えられていない`);
    return next;
  };
  const differs = (next) =>
    JSON.stringify(resolveAll(appCss, next)) !== JSON.stringify(resolveAll(appCss, css));
  assert.ok(differs(mutate("--layer-radius: var(--radius-md) 0 0 0;", "--layer-radius: var(--radius-md);")));
  assert.ok(differs(mutate("--info-idle-dt-opacity: 1;", "--info-idle-dt-opacity: 0.7;")));
  assert.ok(differs(mutate("--row-selected-border-color: var(--accent);", "--row-selected-border-color: var(--row-border);")));
});

test("部品の CSS を変えても、同値の検査は落ちる(トークンを読まなくなった場合)", () => {
  const broken = appCss.replace("border-width: var(--row-border-width);", "border-width: 0 0 1px 0;");
  assert.notEqual(broken, appCss);
  const actual = resolveAll(broken, readAllThemesCss());
  assert.notDeepEqual(actual.fluent.row, EXPECTED.fluent.row);
});

test("旧3テーマの影・等幅フォント・左ペインの初期幅などは、固定値だった時と同じ", () => {
  const themeRules = parseRules(readAllThemesCss());
  for (const theme of LEGACY_THEMES) {
    const tokens = tokensFor(themeRules, theme);
    for (const [name, value] of Object.entries(LEGACY_FIXED_TOKENS)) {
      assert.equal(tokens.get(name), value, `${theme} ${name}`);
    }
  }
});

test("部品の CSS は、影・等幅フォントを固定値でなくトークンから読む", () => {
  assert.doesNotMatch(appCss, /--shadow-[\w-]+:/);
  assert.doesNotMatch(appCss, /--font-mono:/);
  for (const name of ["--shadow-popover", "--shadow-dialog", "--font-mono", "--thumb-gap"]) {
    assert.ok(appCss.includes(`var(${name})`), name);
  }
});
