// DevModeBadge が、設定に従って出し入れされ、保存の通知で更新されることを確かめる。
import { act, render, screen } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import * as api from "../api";
import { DEVELOPER_MODE_CHANGED_EVENT } from "../lib/developerMode";
import { settle } from "../testing/async";
import { DevModeBadge } from "./DevModeBadge";

vi.mock("../api", () => ({
  getDeveloperMode: vi.fn(),
}));

beforeEach(() => {
  vi.spyOn(console, "error").mockImplementation(() => {});
  vi.mocked(api.getDeveloperMode).mockResolvedValue(false);
});

const badge = () => screen.queryByTestId("dev-mode-badge");

test("開発者モードがオフなら、出さない", async () => {
  render(<DevModeBadge />);
  await settle();

  expect(badge()).toBeNull();
});

test("開発者モードがオンなら、出す", async () => {
  vi.mocked(api.getDeveloperMode).mockResolvedValue(true);
  render(<DevModeBadge />);
  await settle();

  expect(badge()).toHaveTextContent("開発者モード");
});

test("設定の保存を知らせるイベントで、取得し直して出し入れする", async () => {
  render(<DevModeBadge />);
  await settle();
  expect(badge()).toBeNull();

  vi.mocked(api.getDeveloperMode).mockResolvedValue(true);
  act(() => {
    window.dispatchEvent(new Event(DEVELOPER_MODE_CHANGED_EVENT));
  });
  await settle();
  expect(badge()).not.toBeNull();

  vi.mocked(api.getDeveloperMode).mockResolvedValue(false);
  act(() => {
    window.dispatchEvent(new Event(DEVELOPER_MODE_CHANGED_EVENT));
  });
  await settle();
  expect(badge()).toBeNull();
});

test("設定を取得できないときは、出さない", async () => {
  vi.mocked(api.getDeveloperMode).mockRejectedValue(new Error("読めない"));
  render(<DevModeBadge />);
  await settle();

  expect(badge()).toBeNull();
});
