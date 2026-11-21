// テーマ(見た目の設計)と外観(ライト/ダーク)の選択肢と、<html> の属性への反映、
// localStorage の写しの読み書き。DOM への依存は引数で受け取る最小の形にして、
// node のテストランナーで検証できるようにしている。
//
// 値の一覧と localStorage のキーは index.html の同期スクリプトにも同じものがある
// (描画前に走るため、このファイルを import できない)。両者の一致は
// src/appearanceIndexHtml.test.tsx が検査する。

export const THEMES = ["graphite", "console", "fluent", "cinema"] as const;
export type Theme = (typeof THEMES)[number];

export const MODES = ["system", "light", "dark"] as const;
export type Mode = (typeof MODES)[number];

// メニューに出す表示名。テーマ名は設計案のタイトル、外観名は OS の用語に揃えた英語表記。
export const THEME_LABELS: Record<Theme, string> = {
  graphite: "Graphite",
  console: "Navy Console",
  fluent: "Fluent Mica",
  cinema: "Dark Cinema",
};

export const MODE_LABELS: Record<Mode, string> = {
  system: "System",
  light: "Light",
  dark: "Dark",
};

export const DEFAULT_THEME: Theme = "graphite";
export const DEFAULT_MODE: Mode = "system";

export const THEME_STORAGE_KEY = "graybrowser.theme";
export const MODE_STORAGE_KEY = "graybrowser.appearanceMode";

export function parseTheme(value: unknown): Theme | null {
  return THEMES.find((t) => t === value) ?? null;
}

export function parseMode(value: unknown): Mode | null {
  return MODES.find((m) => m === value) ?? null;
}

export interface Appearance {
  theme: Theme;
  mode: Mode;
}

export const DEFAULT_APPEARANCE: Appearance = { theme: DEFAULT_THEME, mode: DEFAULT_MODE };

/** `document.documentElement` のうち、属性の反映に使う部分。 */
export interface AttributeTarget {
  dataset: DOMStringMap;
}

export function applyAppearance(root: AttributeTarget, { theme, mode }: Appearance): void {
  root.dataset.theme = theme;
  root.dataset.mode = mode;
}

/** localStorage のうち、写しの読み書きに使う部分。 */
export interface StorageLike {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

// localStorage は無効化・容量超過・アクセス拒否で例外を投げうる。写しは起動時の
// ちらつきを減らすための補助にすぎないので、失敗は黙って既定値/書き込み無しに
// 倒し、画面の表示を止めない。
export function readStoredAppearance(storage: StorageLike | null): Appearance {
  try {
    return {
      theme: parseTheme(storage?.getItem(THEME_STORAGE_KEY)) ?? DEFAULT_THEME,
      mode: parseMode(storage?.getItem(MODE_STORAGE_KEY)) ?? DEFAULT_MODE,
    };
  } catch {
    return DEFAULT_APPEARANCE;
  }
}

export function writeStoredAppearance(
  storage: StorageLike | null,
  { theme, mode }: Appearance,
): void {
  try {
    storage?.setItem(THEME_STORAGE_KEY, theme);
    storage?.setItem(MODE_STORAGE_KEY, mode);
  } catch {
    // 写しが残らないだけ。次回起動は既定の見た目で始まり、DB の値で補正される。
  }
}

/** window.localStorage へのアクセス自体が例外になる環境でも null を返す。 */
export function browserStorage(): StorageLike | null {
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}
