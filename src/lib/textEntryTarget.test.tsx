// DOM の要素を受け取る判定なので、node 組み込みの test runner ではなく vitest(jsdom)で確かめる。
import { expect, test } from "vitest";
import { isTextEditTarget, isTextEntryTarget } from "./textEntryTarget";

function input(type?: string, attrs: Record<string, string> = {}) {
  const el = document.createElement("input");
  if (type !== undefined) el.setAttribute("type", type);
  for (const [k, v] of Object.entries(attrs)) el.setAttribute(k, v);
  return el;
}

/** jsdom は isContentEditable を実装していないので、ブラウザと同じ結果を返すよう定義する。 */
function contentEditable() {
  const el = document.createElement("div");
  Object.defineProperty(el, "isContentEditable", { value: true });
  return el;
}

const TEXT_LIKE = ["text", "search", "url", "tel", "email", "password", "number"];
const NOT_TEXT_LIKE = ["checkbox", "radio", "button", "submit", "range", "color", "file", "date"];

test.each(TEXT_LIKE)("入力欄(type=%s)は、キー入力を受ける対象でも、編集メニューの対象でもある", (type) => {
  expect(isTextEntryTarget(input(type))).toBe(true);
  expect(isTextEditTarget(input(type))).toBe(true);
});

test("type を省略した入力欄は、text として扱う", () => {
  expect(isTextEntryTarget(input())).toBe(true);
  expect(isTextEditTarget(input())).toBe(true);
});

test("未知の type は text に正規化されるので、編集メニューの対象になる", () => {
  expect(isTextEditTarget(input("unknown-type"))).toBe(true);
});

test.each(NOT_TEXT_LIKE)(
  "入力欄(type=%s)は、キー入力を受ける対象だが、編集メニューの対象ではない",
  (type) => {
    expect(isTextEntryTarget(input(type))).toBe(true);
    expect(isTextEditTarget(input(type))).toBe(false);
  },
);

test("読み取り専用・無効の文字入力欄も、コピーできるので編集メニューの対象に含める", () => {
  expect(isTextEditTarget(input("text", { readonly: "" }))).toBe(true);
  expect(isTextEditTarget(input("text", { disabled: "" }))).toBe(true);
});

test("textarea は両方の対象になる", () => {
  const el = document.createElement("textarea");

  expect(isTextEntryTarget(el)).toBe(true);
  expect(isTextEditTarget(el)).toBe(true);
});

test("select はキー入力を受ける対象だが、編集メニューの対象ではない", () => {
  const el = document.createElement("select");

  expect(isTextEntryTarget(el)).toBe(true);
  expect(isTextEditTarget(el)).toBe(false);
});

test("contenteditable の要素は両方の対象になる", () => {
  const el = contentEditable();

  expect(isTextEntryTarget(el)).toBe(true);
  expect(isTextEditTarget(el)).toBe(true);
});

test.each(["div", "button", "a", "span", "body"])("<%s> はどちらの対象でもない", (tag) => {
  const el = document.createElement(tag);

  expect(isTextEntryTarget(el)).toBe(false);
  expect(isTextEditTarget(el)).toBe(false);
});

test("HTMLElement でない対象(window・document・SVG 要素)はどちらの対象でもない", () => {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");

  for (const target of [window, document, svg]) {
    expect(isTextEntryTarget(target)).toBe(false);
    expect(isTextEditTarget(target)).toBe(false);
  }
});

test("HTMLElement でない対象は、編集可能に見える値を持っていてもどちらの対象でもない", () => {
  const fake = Object.assign(new EventTarget(), {
    isContentEditable: true,
    tagName: "INPUT",
    type: "text",
  });

  expect(isTextEntryTarget(fake)).toBe(false);
  expect(isTextEditTarget(fake)).toBe(false);
});

test("input 以外の要素は、type 属性が文字入力らしく見えても編集メニューの対象にしない", () => {
  const link = document.createElement("a");
  link.setAttribute("type", "text");

  expect(isTextEditTarget(link)).toBe(false);
});
