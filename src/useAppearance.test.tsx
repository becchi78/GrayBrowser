// useAppearance: <html> の属性・localStorage の写し・ウィンドウ枠・DB の保存への反映。
import { act, renderHook } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import * as api from "./api";
import { MODE_STORAGE_KEY, THEME_STORAGE_KEY } from "./lib/appearance";
import { deferred, settle } from "./testing/async";
import { useAppearance } from "./useAppearance";

vi.mock("./api", () => ({
  getAppearance: vi.fn(),
  setTheme: vi.fn(),
  setAppearanceMode: vi.fn(),
  setWindowTheme: vi.fn(),
}));

const html = () => document.documentElement;

beforeEach(() => {
  localStorage.clear();
  html().removeAttribute("data-theme");
  html().removeAttribute("data-mode");
  vi.spyOn(console, "error").mockImplementation(() => {});
  vi.mocked(api.getAppearance).mockResolvedValue({ theme: "graphite", mode: "system" });
  vi.mocked(api.setTheme).mockResolvedValue(undefined);
  vi.mocked(api.setAppearanceMode).mockResolvedValue(undefined);
  vi.mocked(api.setWindowTheme).mockResolvedValue(undefined);
});

test("写しが無ければ既定(graphite / system)を属性に付ける", async () => {
  renderHook(() => useAppearance());
  await settle();

  expect(html().dataset.theme).toBe("graphite");
  expect(html().dataset.mode).toBe("system");
});

test("写しがあれば DB の応答を待たずにその値を使う", async () => {
  localStorage.setItem(THEME_STORAGE_KEY, "cinema");
  localStorage.setItem(MODE_STORAGE_KEY, "dark");
  const pending = deferred<{ theme: string; mode: string }>();
  vi.mocked(api.getAppearance).mockReturnValue(pending.promise);

  const { result } = renderHook(() => useAppearance());

  expect(result.current.appearance).toEqual({ theme: "cinema", mode: "dark" });
  expect(html().dataset.theme).toBe("cinema");
  expect(html().dataset.mode).toBe("dark");
  expect(api.setWindowTheme).toHaveBeenCalledWith("dark");
});

test("DB の値が写しと違えば DB の値で補正し、写しも更新する", async () => {
  localStorage.setItem(THEME_STORAGE_KEY, "cinema");
  localStorage.setItem(MODE_STORAGE_KEY, "dark");
  vi.mocked(api.getAppearance).mockResolvedValue({ theme: "fluent", mode: "light" });

  const { result } = renderHook(() => useAppearance());
  await settle();

  expect(result.current.appearance).toEqual({ theme: "fluent", mode: "light" });
  expect(html().dataset.theme).toBe("fluent");
  expect(html().dataset.mode).toBe("light");
  expect(localStorage.getItem(THEME_STORAGE_KEY)).toBe("fluent");
  expect(localStorage.getItem(MODE_STORAGE_KEY)).toBe("light");
  expect(api.setWindowTheme).toHaveBeenLastCalledWith("light");
});

test("DB が未知の値を返したら、その項目は今の値のまま", async () => {
  localStorage.setItem(THEME_STORAGE_KEY, "cinema");
  vi.mocked(api.getAppearance).mockResolvedValue({ theme: "neon", mode: "dark" });

  const { result } = renderHook(() => useAppearance());
  await settle();

  expect(result.current.appearance).toEqual({ theme: "cinema", mode: "dark" });
});

test("DB の取得に失敗しても写しの見た目のまま動く", async () => {
  localStorage.setItem(THEME_STORAGE_KEY, "fluent");
  vi.mocked(api.getAppearance).mockRejectedValue(new Error("x"));

  const { result } = renderHook(() => useAppearance());
  await settle();

  expect(result.current.appearance.theme).toBe("fluent");
  expect(html().dataset.theme).toBe("fluent");
});

test("テーマの選択は属性・写し・DB に反映し、外観は変えない", async () => {
  const { result } = renderHook(() => useAppearance());
  await settle();

  act(() => result.current.selectTheme("cinema"));

  expect(result.current.appearance).toEqual({ theme: "cinema", mode: "system" });
  expect(html().dataset.theme).toBe("cinema");
  expect(html().dataset.mode).toBe("system");
  expect(localStorage.getItem(THEME_STORAGE_KEY)).toBe("cinema");
  expect(api.setTheme).toHaveBeenCalledWith("cinema");
  expect(api.setAppearanceMode).not.toHaveBeenCalled();
});

test("外観の選択は属性・写し・DB・ウィンドウ枠に反映する", async () => {
  const { result } = renderHook(() => useAppearance());
  await settle();

  act(() => result.current.selectMode("dark"));

  expect(result.current.appearance.mode).toBe("dark");
  expect(html().dataset.mode).toBe("dark");
  expect(localStorage.getItem(MODE_STORAGE_KEY)).toBe("dark");
  expect(api.setAppearanceMode).toHaveBeenCalledWith("dark");
  expect(api.setWindowTheme).toHaveBeenLastCalledWith("dark");

  act(() => result.current.selectMode("system"));
  expect(api.setWindowTheme).toHaveBeenLastCalledWith("system");
});

test("同じ外観を選び直してもウィンドウ枠の設定を繰り返さない", async () => {
  const { result } = renderHook(() => useAppearance());
  await settle();
  const before = vi.mocked(api.setWindowTheme).mock.calls.length;

  act(() => result.current.selectMode("system"));

  expect(vi.mocked(api.setWindowTheme).mock.calls.length).toBe(before);
});

test("DB の応答前にユーザーが選んだ項目は、遅れて届いた DB の値で上書きしない", async () => {
  const pending = deferred<{ theme: string; mode: string }>();
  vi.mocked(api.getAppearance).mockReturnValue(pending.promise);
  const { result } = renderHook(() => useAppearance());

  act(() => result.current.selectTheme("fluent"));
  await act(async () => pending.resolve({ theme: "cinema", mode: "dark" }));

  expect(result.current.appearance).toEqual({ theme: "fluent", mode: "dark" });
});

test("localStorage が使えなくても反映は続く", async () => {
  vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
    throw new Error("denied");
  });
  const { result } = renderHook(() => useAppearance());
  await settle();

  act(() => result.current.selectTheme("fluent"));

  expect(result.current.appearance.theme).toBe("fluent");
  expect(html().dataset.theme).toBe("fluent");
});

test("DB の応答前に選んだ外観も、遅れて届いた DB の値で上書きしない(テーマは DB の値になる)", async () => {
  const pending = deferred<{ theme: string; mode: string }>();
  vi.mocked(api.getAppearance).mockReturnValue(pending.promise);
  const { result } = renderHook(() => useAppearance());

  act(() => result.current.selectMode("light"));
  await act(async () => pending.resolve({ theme: "cinema", mode: "dark" }));

  expect(result.current.appearance).toEqual({ theme: "cinema", mode: "light" });
  expect(html().dataset.mode).toBe("light");
});

test("保存に失敗しても、選んだ見た目は反映したまま", async () => {
  vi.mocked(api.setTheme).mockRejectedValue(new Error("db"));
  const { result } = renderHook(() => useAppearance());
  await settle();

  act(() => result.current.selectTheme("fluent"));
  await settle();

  expect(result.current.appearance.theme).toBe("fluent");
  expect(html().dataset.theme).toBe("fluent");
  expect(console.error).toHaveBeenCalled();
});

test("選択関数は再描画をまたいで同一の参照を保つ", async () => {
  const { result } = renderHook(() => useAppearance());
  await settle();
  const { selectTheme, selectMode } = result.current;

  act(() => result.current.selectTheme("cinema"));

  expect(result.current.selectTheme).toBe(selectTheme);
  expect(result.current.selectMode).toBe(selectMode);
});
