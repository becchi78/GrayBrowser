// テーマの構成の検査。package.json の test:unit 参照。
// - 必須トークンの欠落
// - テーマ名の一覧(マニフェスト・TS・Rust・index.html・CSS のファイル・メニュー)の一致
// - [data-theme セレクタの置き場所(src/themes/ の外には書かない)
import assert from "node:assert/strict";
import { readdirSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import { test } from "node:test";
import { DEFAULT_THEME, MODES, THEMES, THEME_LABELS } from "../src/lib/appearance.ts";
import {
  SRC_DIR,
  missingRequired,
  readManifest,
  readText,
  readThemeCss,
  selectorOverrides,
  stripComments,
  themeFileNames,
} from "./theme-css.mjs";

const ROOT = join(SRC_DIR, "..");
const read = (rel) => readText(join(ROOT, rel));
const manifest = readManifest();

// ---- 必須トークン ----

test("全テーマが、必須のトークンとプロパティをすべて定義している", () => {
  assert.ok(manifest.requiredTokens.length > 0);
  for (const theme of manifest.themes) {
    assert.deepEqual(missingRequired(readThemeCss(theme), theme, manifest), [], theme);
  }
});

test("必須トークンを1つ削ると欠落として検出される(既存・追加どちらのトークンも)", () => {
  for (const token of ["--accent", "--row-border-width", "--list-header-letter-spacing"]) {
    const css = readThemeCss("fluent");
    const cut = css.replace(new RegExp(`^\\s*${token}:[^;]*;\\n`, "m"), "");
    assert.notEqual(cut, css, `${token} を削れていない`);
    assert.deepEqual(missingRequired(cut, "fluent", manifest), [token]);
  }
  const noFont = readThemeCss("cinema").replace(/^\s*font-size:[^;]*;\n/m, "");
  assert.deepEqual(missingRequired(noFont, "cinema", manifest), ["font-size"]);
});

test("別のテーマのセレクタで定義したトークンは、そのテーマの定義に数えない", () => {
  const css = readThemeCss("fluent").replaceAll('data-theme="fluent"', 'data-theme="cinema"');
  assert.ok(missingRequired(css, "fluent", manifest).length > 0);
});

test("必須トークンの一覧に重複が無く、_base が導出するトークンを含まない", () => {
  const tokens = manifest.requiredTokens;
  assert.equal(new Set(tokens).size, tokens.length);
  const base = stripComments(read("src/themes/_base.css"));
  const derived = new Set([...base.matchAll(/^\s*(--[\w-]+):/gm)].map((m) => m[1]));
  for (const t of ["--border-muted", "--thumbnail-bg", "--link", "--info-w", "--thumb-w"]) {
    assert.ok(derived.has(t), `${t} は _base の導出`);
    assert.ok(!tokens.includes(t), `${t} は必須に入れない`);
  }
});

test("素の :root は既定テーマのファイルだけが持てる(他のテーマに書くと全テーマに効くので検出する)", () => {
  assert.equal(manifest.defaultTheme, DEFAULT_THEME);
  assert.ok(manifest.themes.includes(manifest.defaultTheme));
  // 既定テーマ(console)の `:root, :root[data-theme="console"]` は許可される。
  const consoleCss = readThemeCss("console");
  assert.match(consoleCss, /:root,\s*:root\[data-theme="console"\]/);
  assert.deepEqual(selectorOverrides(consoleCss, "console", manifest), []);
  assert.deepEqual(missingRequired(consoleCss, "console", manifest), []);
  // 他のテーマのファイルに素の :root を足すと、セレクタ上書きとして検出され、
  // その中のトークンは、そのテーマの定義には数えない。
  for (const theme of manifest.themes.filter((t) => t !== manifest.defaultTheme)) {
    const css = `${readThemeCss(theme)}\n:root { --accent: red; }\n`;
    assert.deepEqual(selectorOverrides(css, theme, manifest), [":root"], theme);
    const without = readThemeCss(theme).replace(/^\s*--accent:[^;]*;\n/m, "");
    const sneaky = `${without}\n:root { --accent: red; }\n`;
    assert.deepEqual(missingRequired(sneaky, theme, manifest), ["--accent"], theme);
  }
});

test("テーマのファイルに、トークン以外のセレクタ上書きは無い", () => {
  for (const theme of manifest.themes) {
    assert.deepEqual(selectorOverrides(readThemeCss(theme), theme), [], theme);
  }
  const bad = `${readThemeCss("fluent")}\n.video-row { color: red; }\n`;
  assert.deepEqual(selectorOverrides(bad, "fluent"), [".video-row"]);
});

// ---- テーマ名の一致 ----

/** Rust の Theme::ALL(並び)と as_str の対応から、文字列の並びを読む。 */
export function rustThemeNames(rs) {
  const all = /pub const ALL: \[Theme; \d+\] = \[([^\]]*)\]/.exec(rs);
  assert.ok(all, "Theme::ALL が見つからない");
  const strs = new Map(
    [...rs.matchAll(/Theme::(\w+)\s*=>\s*"([^"]+)"/g)].map((m) => [m[1], m[2]]),
  );
  return [...all[1].matchAll(/Theme::(\w+)/g)].map((m) => {
    const s = strs.get(m[1]);
    assert.ok(s !== undefined, `Theme::${m[1]} に as_str の対応が無い`);
    return s;
  });
}

function htmlList(html, name) {
  const m = new RegExp(`var ${name} = \\[([^\\]]*)\\]`).exec(html);
  assert.ok(m, `index.html に ${name} の配列が無い`);
  return [...m[1].matchAll(/"([^"]+)"/g)].map((x) => x[1]);
}

/** メニューが、テーマ一覧から `theme-<名前>` の id で項目を作っていれば、その id の並び。 */
export function menuThemeIdsFromSource(menuSource) {
  const ok =
    /THEMES\.map\(\(t\) =>/.test(menuSource) &&
    menuSource.includes("id: `theme-${t}`") &&
    menuSource.includes("THEME_LABELS[t]");
  return ok ? [...THEMES].map((t) => `theme-${t}`) : null;
  // null のとき: AppMenu.tsx の書き方が変わった(リファクタ)のか、テーマの項目が
  // 漏れたのかを、実際の項目 id を見る src/components/AppMenu.test.tsx と見比べて判断する。
}

/** 各所のテーマ名の一覧を突き合わせ、食い違いの説明を返す(空なら一致)。 */
export function themeNameMismatches(src) {
  const out = [];
  const same = (label, actual, unordered = false) => {
    const norm = (a) => JSON.stringify(unordered ? [...a].sort() : a);
    if (!actual || norm(actual) !== norm(src.manifest)) {
      out.push(`${label}: ${JSON.stringify(actual)} <> manifest ${JSON.stringify(src.manifest)}`);
    }
  };
  same("appearance.ts THEMES", src.ts);
  same("appearance.rs Theme::ALL", src.rust);
  same("index.html themes", src.html);
  same("THEME_LABELS のキー", src.labels);
  same(
    "AppMenu の項目 id(null は AppMenu.tsx のソースの書き方が検査の想定と違う。リファクタなら menuThemeIdsFromSource を直し、テーマの漏れなら AppMenu.tsx と AppMenu.test.tsx を確認する)",
    src.menuIds?.map((id) => id.replace(/^theme-/, "")),
  );
  same("src/themes/ のファイル", src.files, true);
  return out;
}

function realSources() {
  return {
    manifest: manifest.themes,
    ts: [...THEMES],
    rust: rustThemeNames(read("crates/gb-core/src/appearance.rs")),
    html: htmlList(read("index.html"), "themes"),
    labels: Object.keys(THEME_LABELS),
    menuIds: menuThemeIdsFromSource(read("src/components/AppMenu.tsx")),
    files: themeFileNames(),
  };
}

test("テーマ名は、マニフェスト・TS・Rust・index.html・ラベル・メニュー・CSS のファイルで一致する", () => {
  assert.deepEqual(themeNameMismatches(realSources()), []);
});

test("外観(mode)の一覧は index.html と appearance.ts で一致する", () => {
  assert.deepEqual(htmlList(read("index.html"), "modes"), [...MODES]);
});

test("どこか1か所でも食い違えば検出される", () => {
  const base = realSources();
  const added = [...base.manifest, "graphite"];
  assert.ok(themeNameMismatches({ ...base, manifest: added }).length > 0);
  assert.equal(themeNameMismatches({ ...base, html: ["console", "fluent"] }).length, 1);
  assert.equal(themeNameMismatches({ ...base, rust: [...base.rust, "graphite"] }).length, 1);
  assert.equal(themeNameMismatches({ ...base, ts: ["fluent", "console", "cinema"] }).length, 1);
  assert.equal(themeNameMismatches({ ...base, labels: ["console", "fluent"] }).length, 1);
  assert.equal(themeNameMismatches({ ...base, files: ["console", "fluent"] }).length, 1);
  assert.equal(themeNameMismatches({ ...base, menuIds: null }).length, 1);
});

test("Rust のソースの読み取りは、as_str に対応が無いバリアントを検出する", () => {
  const rs = `pub const ALL: [Theme; 2] = [Theme::A, Theme::B];
    fn as_str(self) { match self { Theme::A => "a" } }`;
  assert.throws(() => rustThemeNames(rs), /Theme::B/);
  assert.deepEqual(
    rustThemeNames(`pub const ALL: [Theme; 1] = [Theme::A]; match self { Theme::A => "a" }`),
    ["a"],
  );
});

// ---- [data-theme セレクタの置き場所 ----

/** src 以下の CSS ファイル(ROOT からの相対パス、/ 区切り)。 */
function cssFiles(dir = SRC_DIR) {
  return readdirSync(dir).flatMap((name) => {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) return cssFiles(p);
    return name.endsWith(".css") ? [relative(ROOT, p).replaceAll("\\", "/")] : [];
  });
}

export function usesThemeSelector(css) {
  return /\[\s*data-theme/.test(stripComments(css));
}

test("[data-theme セレクタは src/themes/ の CSS にだけある(部品の CSS は書かない)", () => {
  const files = cssFiles();
  assert.ok(files.includes("src/App.css") && files.includes("src/themes.css"));
  for (const f of files) {
    if (f.startsWith("src/themes/")) continue;
    assert.equal(usesThemeSelector(read(f)), false, `${f} に [data-theme がある`);
  }
  // テーマのファイルには(許可された置き場所として)ある。
  assert.ok(manifest.themes.every((t) => usesThemeSelector(readThemeCss(t))));
});

test("セレクタの検出は、コメントの中の言及では反応せず、実際の書式には反応する", () => {
  assert.equal(usesThemeSelector('/* :root[data-theme="x"] */ .a { color: red; }'), false);
  assert.equal(usesThemeSelector(':root[data-theme="x"] .a { color: red; }'), true);
  assert.equal(usesThemeSelector(':root[ data-theme="x"] .a { color: red; }'), true);
  assert.equal(usesThemeSelector(":root[\n  data-theme=x] .a { color: red; }"), true);
});

test("src/themes.css は、_base とマニフェストの全テーマを、この順で @import する", () => {
  const imports = [...read("src/themes.css").matchAll(/@import "\.\/themes\/([^"]+)\.css";/g)].map(
    (m) => m[1],
  );
  assert.deepEqual(imports, ["_base", ...manifest.themes]);
});
