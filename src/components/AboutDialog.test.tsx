// AboutDialog が、開発者モードがオンのときだけ「この版の状態」を出すことを確かめる。
import { screen } from "@testing-library/react";
import { getVersion } from "@tauri-apps/api/app";
import { beforeEach, expect, test, vi } from "vitest";
import * as api from "../api";
import { deferred, never, settle } from "../testing/async";
import { renderWithProviders } from "../testing/render";
import { AboutDialog } from "./AboutDialog";

vi.mock("../api", () => ({
  getDeveloperMode: vi.fn(),
  getReleaseStatus: vi.fn(),
}));

beforeEach(() => {
  vi.spyOn(console, "error").mockImplementation(() => {});
  vi.mocked(getVersion).mockResolvedValue("1.2.3");
  vi.mocked(api.getDeveloperMode).mockResolvedValue(true);
  vi.mocked(api.getReleaseStatus).mockResolvedValue("published");
});

async function renderDialog() {
  renderWithProviders(<AboutDialog open={true} onClose={vi.fn()} />);
  await settle();
}

const status = () => screen.queryByTestId("about-dialog-release-status");

test("開発者モードがオフなら、この版の状態を出さず、取得もしない", async () => {
  vi.mocked(api.getDeveloperMode).mockResolvedValue(false);

  await renderDialog();

  expect(status()).toBeNull();
  expect(api.getReleaseStatus).not.toHaveBeenCalled();
});

test("開発者モードがオンで、公開前の版なら「Draft版」と出す", async () => {
  vi.mocked(api.getReleaseStatus).mockResolvedValue("preview");

  await renderDialog();

  expect(status()).toHaveTextContent("この版の状態: Draft版");
});

test("開発者モードがオンで、公開済みの版なら「公開済みの安定版」と出す", async () => {
  await renderDialog();

  expect(status()).toHaveTextContent("この版の状態: 公開済みの安定版");
});

test("取得できなかったときは「確認できません」と出す", async () => {
  vi.mocked(api.getReleaseStatus).mockResolvedValue("unknown");

  await renderDialog();

  expect(status()).toHaveTextContent("この版の状態: 確認できません");
});

test("取得がエラーになっても、ダイアログは壊れず「確認できません」と出す", async () => {
  vi.mocked(api.getReleaseStatus).mockRejectedValue(new Error("拒否された"));

  await renderDialog();

  expect(screen.getByTestId("about-dialog-version")).toHaveTextContent("バージョン 1.2.3");
  expect(status()).toHaveTextContent("この版の状態: 確認できません");
});

test("取得の途中は「確認中」と出す", async () => {
  vi.mocked(api.getReleaseStatus).mockReturnValue(never());

  await renderDialog();

  expect(status()).toHaveTextContent("この版の状態: 確認中...");
});

test("取得が終わる前に閉じても、状態更新の警告が出ない", async () => {
  const pending = deferred<api.ReleaseStatus>();
  vi.mocked(api.getReleaseStatus).mockReturnValue(pending.promise);
  const errorSpy = vi.spyOn(console, "error").mockImplementation(() => {});

  const view = renderWithProviders(<AboutDialog open={true} onClose={vi.fn()} />);
  await settle();
  view.unmount();
  pending.resolve("preview");
  await settle();

  expect(errorSpy).not.toHaveBeenCalled();
});
