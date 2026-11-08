// useEscapeKey の現状の動作を固定する特性テスト。
import { expect, test, vi } from "vitest";
import { render } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useEscapeKey } from "./useEscapeKey";

function Harness({ onEscape, enabled }: { onEscape: () => void; enabled?: boolean }) {
  useEscapeKey(onEscape, enabled);
  return null;
}

function pressKey(key: string): KeyboardEvent {
  // 既定動作の抑止(preventDefault)を見るため、キャンセル可能なイベントを自前で送る。
  const event = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true });
  document.dispatchEvent(event);
  return event;
}

test("enabled が true のとき Escape でコールバックを呼び、既定動作を止める", () => {
  const onEscape = vi.fn();
  render(<Harness onEscape={onEscape} enabled={true} />);

  const event = pressKey("Escape");

  expect(onEscape).toHaveBeenCalledTimes(1);
  expect(event.defaultPrevented).toBe(true);
});

test("enabled を省略すると有効として扱う", () => {
  const onEscape = vi.fn();
  render(<Harness onEscape={onEscape} />);

  pressKey("Escape");

  expect(onEscape).toHaveBeenCalledTimes(1);
});

test("enabled が false のときはコールバックを呼ばず、既定動作も止めない", () => {
  const onEscape = vi.fn();
  render(<Harness onEscape={onEscape} enabled={false} />);

  const event = pressKey("Escape");

  expect(onEscape).not.toHaveBeenCalled();
  expect(event.defaultPrevented).toBe(false);
});

test("Escape 以外のキーではコールバックを呼ばず、既定動作も止めない", () => {
  const onEscape = vi.fn();
  render(<Harness onEscape={onEscape} enabled={true} />);

  const event = pressKey("Enter");

  expect(onEscape).not.toHaveBeenCalled();
  expect(event.defaultPrevented).toBe(false);
});

test("アンマウント後の Escape ではコールバックを呼ばない", () => {
  const onEscape = vi.fn();
  const { unmount } = render(<Harness onEscape={onEscape} enabled={true} />);

  unmount();
  pressKey("Escape");

  expect(onEscape).not.toHaveBeenCalled();
});

test("enabled が false から true に変わると購読を始め、true から false で止める", () => {
  const onEscape = vi.fn();
  const { rerender } = render(<Harness onEscape={onEscape} enabled={false} />);

  rerender(<Harness onEscape={onEscape} enabled={true} />);
  pressKey("Escape");
  expect(onEscape).toHaveBeenCalledTimes(1);

  rerender(<Harness onEscape={onEscape} enabled={false} />);
  pressKey("Escape");
  expect(onEscape).toHaveBeenCalledTimes(1);
});

test("コールバックを差し替えると、以降は新しい方だけが呼ばれる", () => {
  const first = vi.fn();
  const second = vi.fn();
  const { rerender } = render(<Harness onEscape={first} enabled={true} />);

  rerender(<Harness onEscape={second} enabled={true} />);
  pressKey("Escape");

  expect(first).not.toHaveBeenCalled();
  expect(second).toHaveBeenCalledTimes(1);
});

test("userEvent のキー入力でも Escape で呼ばれる", async () => {
  const user = userEvent.setup();
  const onEscape = vi.fn();
  render(<Harness onEscape={onEscape} enabled={true} />);

  await user.keyboard("{Escape}");

  expect(onEscape).toHaveBeenCalledTimes(1);
});
