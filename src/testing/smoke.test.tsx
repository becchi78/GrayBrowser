import { useState } from "react";
import { describe, expect, it, vi } from "vitest";
import { screen } from "@testing-library/react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { renderWithProviders } from "./render";

function Counter() {
  const [n, setN] = useState(0);
  return <button onClick={() => setN(n + 1)}>count: {n}</button>;
}

describe("テスト基盤", () => {
  it("jsdom で描画して操作を検証できる", async () => {
    const { user } = renderWithProviders(<Counter />);
    await user.click(screen.getByRole("button", { name: "count: 0" }));
    expect(screen.getByRole("button")).toHaveTextContent("count: 1");
  });

  it("未モックの invoke / listen は例外になる", () => {
    expect(() => invoke("x")).toThrow("未モックの Tauri 呼び出し");
    expect(() => listen("x", () => {})).toThrow("未モックの Tauri 呼び出し");
  });

  it("テストが明示的に上書きした invoke は動き、次のテストには持ち越されない", async () => {
    vi.mocked(invoke).mockResolvedValueOnce("ok");
    await expect(invoke("x")).resolves.toBe("ok");
    expect(() => invoke("x")).toThrow();
  });
});
