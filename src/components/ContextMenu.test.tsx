import { expect, test, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { ContextMenu, type ContextMenuItem } from "./ContextMenu";

function renderMenu(items: ContextMenuItem[]) {
  return render(<ContextMenu position={{ x: 10, y: 10 }} items={items} onClose={vi.fn()} />);
}

const items = (): ContextMenuItem[] => [
  { label: "A", onSelect: vi.fn() },
  { label: "B", onSelect: vi.fn() },
  { label: "C", disabled: true, onSelect: vi.fn() },
];

test("開いた直後は先頭の項目にフォーカスがある", () => {
  renderMenu(items());
  expect(screen.getByRole("menuitem", { name: "A" })).toHaveFocus();
});

test("マウスが乗った項目へフォーカスが移る", () => {
  renderMenu(items());
  fireEvent.mouseEnter(screen.getByRole("menuitem", { name: "B" }));
  expect(screen.getByRole("menuitem", { name: "B" })).toHaveFocus();
});

test("disabled の項目に乗ってもフォーカスは移らない", () => {
  renderMenu(items());
  fireEvent.mouseEnter(screen.getByRole("menuitem", { name: "B" }));
  fireEvent.mouseEnter(screen.getByRole("menuitem", { name: "C" }));
  expect(screen.getByRole("menuitem", { name: "B" })).toHaveFocus();
});

test("ホバー後も矢印キーはフォーカス中の項目から移動する", () => {
  renderMenu(items());
  fireEvent.mouseEnter(screen.getByRole("menuitem", { name: "B" }));
  fireEvent.keyDown(document, { key: "ArrowDown" });
  expect(screen.getByRole("menuitem", { name: "A" })).toHaveFocus();
});
