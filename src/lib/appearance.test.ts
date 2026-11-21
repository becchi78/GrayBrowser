// appearance のユニットテスト。package.json の test:unit 参照。
import assert from "node:assert/strict";
import { test } from "node:test";
import {
  DEFAULT_APPEARANCE,
  MODE_STORAGE_KEY,
  THEME_STORAGE_KEY,
  applyAppearance,
  parseMode,
  parseTheme,
  readStoredAppearance,
  writeStoredAppearance,
  type StorageLike,
} from "./appearance.ts";

function memoryStorage(initial: Record<string, string> = {}): StorageLike & {
  data: Record<string, string>;
} {
  const data = { ...initial };
  return {
    data,
    getItem: (k) => (k in data ? data[k] : null),
    setItem: (k, v) => {
      data[k] = v;
    },
  };
}

const throwingStorage: StorageLike = {
  getItem: () => {
    throw new Error("denied");
  },
  setItem: () => {
    throw new Error("denied");
  },
};

test("既定は console / system", () => {
  assert.deepEqual(DEFAULT_APPEARANCE, { theme: "console", mode: "system" });
});

test("parseTheme / parseMode は既知の値だけを受け付ける", () => {
  assert.equal(parseTheme("fluent"), "fluent");
  assert.equal(parseTheme("cinema"), "cinema");
  assert.equal(parseTheme("console"), "console");
  assert.equal(parseMode("system"), "system");
  assert.equal(parseMode("light"), "light");
  assert.equal(parseMode("dark"), "dark");
  for (const bad of ["", "Fluent", "neon", " dark", null, undefined, 1, {}]) {
    assert.equal(parseTheme(bad), null, String(bad));
    assert.equal(parseMode(bad), null, String(bad));
  }
});

test("applyAppearance は data-theme / data-mode を付ける", () => {
  const root = { dataset: {} as DOMStringMap };
  applyAppearance(root, { theme: "cinema", mode: "dark" });
  assert.equal(root.dataset.theme, "cinema");
  assert.equal(root.dataset.mode, "dark");
  applyAppearance(root, { theme: "fluent", mode: "system" });
  assert.equal(root.dataset.theme, "fluent");
  assert.equal(root.dataset.mode, "system");
});

test("写しが無ければ既定値を返す", () => {
  assert.deepEqual(readStoredAppearance(memoryStorage()), DEFAULT_APPEARANCE);
  assert.deepEqual(readStoredAppearance(null), DEFAULT_APPEARANCE);
});

test("書いた値を読み戻せる", () => {
  const storage = memoryStorage();
  writeStoredAppearance(storage, { theme: "fluent", mode: "light" });
  assert.equal(storage.data[THEME_STORAGE_KEY], "fluent");
  assert.equal(storage.data[MODE_STORAGE_KEY], "light");
  assert.deepEqual(readStoredAppearance(storage), { theme: "fluent", mode: "light" });
});

test("不正な値は項目ごとに既定値へ倒す", () => {
  const storage = memoryStorage({ [THEME_STORAGE_KEY]: "neon", [MODE_STORAGE_KEY]: "dark" });
  assert.deepEqual(readStoredAppearance(storage), { theme: "console", mode: "dark" });
  const storage2 = memoryStorage({ [THEME_STORAGE_KEY]: "cinema", [MODE_STORAGE_KEY]: "" });
  assert.deepEqual(readStoredAppearance(storage2), { theme: "cinema", mode: "system" });
});

test("localStorage が例外を投げても読み書きは失敗せず、既定値を返す", () => {
  assert.deepEqual(readStoredAppearance(throwingStorage), DEFAULT_APPEARANCE);
  assert.doesNotThrow(() => writeStoredAppearance(throwingStorage, DEFAULT_APPEARANCE));
});
