// DevModeBadge が、設定に従って出し入れされ、保存の通知で更新されることを確かめる。
import { act, render, screen } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import * as api from "../api";
import { DEVELOPER_MODE_CHANGED_EVENT } from "../lib/developerMode";
import { settle } from "../testing/async";
import { DevModeBadge } from "./DevModeBadge";

vi.mock("../api", () => ({
  getDeveloperMode: vi.fn(),
  getDataDirInfo: vi.fn(),
}));

beforeEach(() => {
  vi.spyOn(console, "error").mockImplementation(() => {});
  vi.mocked(api.getDeveloperMode).mockResolvedValue(false);
  vi.mocked(api.getDataDirInfo).mockResolvedValue({
    path: String.raw`%LOCALAPPDATA%\GrayBrowser`,
    overridden: false,
  });
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

const dataDir = () => screen.queryByTestId("data-dir-indicator");

test("開発者モードがオンなら、使用中のデータフォルダを出す(既定のときは強調しない)", async () => {
  vi.mocked(api.getDeveloperMode).mockResolvedValue(true);
  render(<DevModeBadge />);
  await settle();

  expect(dataDir()).toHaveTextContent(String.raw`データフォルダ: %LOCALAPPDATA%\GrayBrowser`);
  expect(dataDir()).not.toHaveClass("data-dir-indicator-overridden");
});

test("別のフォルダを指定して起動しているときは、強調して出す", async () => {
  vi.mocked(api.getDeveloperMode).mockResolvedValue(true);
  vi.mocked(api.getDataDirInfo).mockResolvedValue({ path: String.raw`D:\gb-test`, overridden: true });
  render(<DevModeBadge />);
  await settle();

  expect(dataDir()).toHaveTextContent(String.raw`別のデータフォルダ: D:\gb-test`);
  expect(dataDir()).toHaveClass("data-dir-indicator-overridden");
});

test("開発者モードがオフなら、データフォルダを取得も表示もしない", async () => {
  render(<DevModeBadge />);
  await settle();

  expect(dataDir()).toBeNull();
  expect(api.getDataDirInfo).not.toHaveBeenCalled();
});

test("データフォルダを取得できなくても、バッジは出す", async () => {
  vi.mocked(api.getDeveloperMode).mockResolvedValue(true);
  vi.mocked(api.getDataDirInfo).mockRejectedValue(new Error("読めない"));
  render(<DevModeBadge />);
  await settle();

  expect(dataDir()).toBeNull();
  expect(badge()).not.toBeNull();
});
