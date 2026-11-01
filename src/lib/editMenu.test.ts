import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { editMenuState, type EditMenuInput } from "./editMenu.ts";

const base: EditMenuInput = {
  value: "hello",
  selectionStart: 1,
  selectionEnd: 3,
  readOnly: false,
  disabled: false,
  isPassword: false,
};

describe("editMenuState", () => {
  it("enables everything for an editable field with a selection", () => {
    assert.deepEqual(editMenuState(base), {
      cut: true,
      copy: true,
      paste: true,
      selectAll: true,
    });
  });

  it("disables cut and copy without a selection", () => {
    const s = editMenuState({ ...base, selectionStart: 2, selectionEnd: 2 });
    assert.equal(s.cut, false);
    assert.equal(s.copy, false);
    assert.equal(s.paste, true);
    assert.equal(s.selectAll, true);
  });

  it("disables select-all on an empty field", () => {
    const s = editMenuState({ ...base, value: "", selectionStart: 0, selectionEnd: 0 });
    assert.deepEqual(s, { cut: false, copy: false, paste: true, selectAll: false });
  });

  it("read-only keeps copy and select-all but not cut or paste", () => {
    const s = editMenuState({ ...base, readOnly: true });
    assert.deepEqual(s, { cut: false, copy: true, paste: false, selectAll: true });
  });

  it("disabled behaves like read-only", () => {
    const s = editMenuState({ ...base, disabled: true });
    assert.deepEqual(s, { cut: false, copy: true, paste: false, selectAll: true });
  });

  it("password fields cannot cut or copy but can paste and select all", () => {
    const s = editMenuState({ ...base, isPassword: true });
    assert.deepEqual(s, { cut: false, copy: false, paste: true, selectAll: true });
  });

  it("treats a reversed range as no selection", () => {
    const s = editMenuState({ ...base, selectionStart: 3, selectionEnd: 1 });
    assert.equal(s.copy, false);
  });
});
