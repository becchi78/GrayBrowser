// メニューから開くダイアログの排他と、「閉じられない間は開く要求を無視する」判定を確かめる。
import { act, fireEvent, render, renderHook, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import * as api from "./api";
import { UpdateNotice } from "./components/UpdateNotice";
import { never } from "./testing/async";
import { fakeUpdate } from "./testing/fakes";
import { registerUndismissibleModal } from "./undismissibleModals";
import { useActiveDialog, type DialogKind } from "./useActiveDialog";

vi.mock("./api", () => ({
  checkForUpdate: vi.fn(),
  getUpdateCheckOnStartup: vi.fn(),
}));

const KINDS: DialogKind[] = ["folder", "wbImport", "tagBar", "about", "log", "settings", "update"];

const releases: Array<() => void> = [];
function blockDismissal() {
  releases.push(registerUndismissibleModal());
}
afterEach(() => {
  releases.splice(0).forEach((release) => release());
});

test("最初はどのダイアログも開いていない", () => {
  const { result } = renderHook(() => useActiveDialog());
  expect(result.current.active).toBeNull();
});

test.each(KINDS.flatMap((from) => KINDS.map((to) => [from, to] as const)))(
  "%s が開いているときに %s を開くと、%s は閉じて %s だけが開く",
  (from, to) => {
    const { result } = renderHook(() => useActiveDialog());
    act(() => result.current.open(from));
    act(() => result.current.open(to));
    expect(result.current.active).toBe(to);
  },
);

test("同じダイアログを続けて開いても開いたまま", () => {
  const { result } = renderHook(() => useActiveDialog());
  act(() => result.current.open("log"));
  act(() => result.current.open("log"));
  expect(result.current.active).toBe("log");
});

test("閉じると何も開いていない状態に戻る", () => {
  const { result } = renderHook(() => useActiveDialog());
  act(() => result.current.open("about"));
  act(() => result.current.close("about"));
  expect(result.current.active).toBeNull();
});

test("閉じられないダイアログが開いている間は、別のダイアログを開く要求を無視する", () => {
  const { result } = renderHook(() => useActiveDialog());
  act(() => result.current.open("wbImport"));
  blockDismissal();

  act(() => result.current.open("settings"));

  expect(result.current.active).toBe("wbImport");
});

test("閉じられないダイアログが閉じられる状態に戻れば、再び切り替えられる", () => {
  const { result } = renderHook(() => useActiveDialog());
  act(() => result.current.open("wbImport"));
  const release = registerUndismissibleModal();
  act(() => result.current.open("settings"));
  release();

  act(() => result.current.open("settings"));

  expect(result.current.active).toBe("settings");
});

test("すでに別のダイアログへ切り替わった後に古い onClose が呼ばれても、新しいダイアログは閉じない", () => {
  const { result } = renderHook(() => useActiveDialog());
  act(() => result.current.open("folder"));
  act(() => result.current.open("about"));

  act(() => result.current.close("folder"));

  expect(result.current.active).toBe("about");
});

// ---- 更新通知から更新ダイアログを開く ----

function Harness() {
  const { active, open } = useActiveDialog();
  return (
    <>
      <button type="button" data-testid="open-log" onClick={() => open("log")} />
      <span data-testid="active">{active ?? "none"}</span>
      <UpdateNotice onOpenDetails={() => open("update")} />
    </>
  );
}

beforeEach(() => {
  vi.useFakeTimers();
  vi.spyOn(console, "warn").mockImplementation(() => {});
  vi.mocked(api.getUpdateCheckOnStartup).mockResolvedValue(true);
  vi.mocked(api.checkForUpdate).mockResolvedValue(fakeUpdate(() => never()));
});

afterEach(() => {
  vi.useRealTimers();
});

test("更新通知の「詳細・更新」で、開いていた別のダイアログに替わって更新ダイアログが開く", async () => {
  render(<Harness />);
  fireEvent.click(screen.getByTestId("open-log"));
  expect(screen.getByTestId("active")).toHaveTextContent("log");
  await act(async () => {
    await vi.advanceTimersByTimeAsync(5000);
  });

  fireEvent.click(screen.getByTestId("update-notice-details-btn"));

  expect(screen.getByTestId("active")).toHaveTextContent("update");
});
