// VersionPinDialog の、一覧→選択→確認→バックアップ→インストールの流れと、警告の出方・
// バックアップに失敗したときに進まないことを確かめる。
import { screen } from "@testing-library/react";
import { getVersion } from "@tauri-apps/api/app";
import { beforeEach, expect, test, vi } from "vitest";
import * as api from "../api";
import { deferred, never, settle } from "../testing/async";
import { fakeUpdate } from "../testing/fakes";
import { renderWithProviders } from "../testing/render";
import { VersionPinDialog } from "./VersionPinDialog";

vi.mock("../api", () => ({
  listInstallableReleases: vi.fn(),
  prepareVersionInstall: vi.fn(),
  backupBeforeInstall: vi.fn(),
}));

const RELEASES: api.InstallableRelease[] = [
  { version: "1.2.0", publishedAt: "2026-11-20T00:00:00Z" },
  { version: "1.1.0", publishedAt: "2026-11-10T00:00:00Z" },
];

function prepared(
  overrides: Partial<Omit<api.PreparedInstall, "update">> = {},
  downloadAndInstall: Parameters<typeof fakeUpdate>[0] = () => never<void>(),
): api.PreparedInstall {
  return {
    update: fakeUpdate(downloadAndInstall, { version: "1.1.0" }),
    isDowngrade: true,
    backupRequired: true,
    schemaWarning: null,
    currentSchemaVersion: 13,
    targetSchemaVersion: 13,
    ...overrides,
  };
}

beforeEach(() => {
  vi.spyOn(console, "error").mockImplementation(() => {});
  vi.mocked(getVersion).mockResolvedValue("1.2.0");
  vi.mocked(api.listInstallableReleases).mockResolvedValue(RELEASES);
  vi.mocked(api.backupBeforeInstall).mockResolvedValue("app_manual.db");
});

async function renderDialog() {
  const onClose = vi.fn();
  const view = renderWithProviders(<VersionPinDialog open={true} onClose={onClose} />);
  await settle();
  return { ...view, onClose };
}

async function choose(user: Awaited<ReturnType<typeof renderDialog>>["user"], version: string) {
  await user.click(screen.getByTestId(`version-pin-item-${version}`));
  await settle();
}

test("閉じているときは何も出さず、取得もしない", () => {
  renderWithProviders(<VersionPinDialog open={false} onClose={vi.fn()} />);

  expect(screen.queryByTestId("version-pin-dialog")).toBeNull();
  expect(api.listInstallableReleases).not.toHaveBeenCalled();
});

test("一覧を出し、現在のバージョンに印を付ける", async () => {
  await renderDialog();

  expect(screen.getByTestId("version-pin-item-1.2.0")).toHaveTextContent("現在");
  expect(screen.getByTestId("version-pin-item-1.1.0")).not.toHaveTextContent("現在");
  expect(screen.getByTestId("version-pin-item-1.2.0")).toHaveTextContent("2026-11-20");
});

test("一覧を取得できないとエラーを出し、再試行で取り直せる", async () => {
  vi.mocked(api.listInstallableReleases).mockRejectedValueOnce(new Error("オフライン"));
  const { user } = await renderDialog();
  expect(screen.getByTestId("version-pin-error")).toHaveTextContent("オフライン");

  await user.click(screen.getByTestId("version-pin-retry-btn"));
  await settle();

  expect(screen.getByTestId("version-pin-item-1.1.0")).toBeInTheDocument();
});

test("リリースが無いときはその旨を出す", async () => {
  vi.mocked(api.listInstallableReleases).mockResolvedValue([]);
  await renderDialog();

  expect(screen.getByTestId("version-pin-empty")).toBeInTheDocument();
});

test("選んだ版の確認では、ダウングレードの自動バックアップとスキーマの注意を明記する", async () => {
  vi.mocked(api.prepareVersionInstall).mockResolvedValue(prepared());
  const { user } = await renderDialog();

  await choose(user, "1.1.0");

  expect(api.prepareVersionInstall).toHaveBeenCalledWith("1.1.0");
  const notice = screen.getByTestId("version-pin-downgrade-notice");
  expect(notice).toHaveTextContent("データベースを自動でバックアップします");
  expect(notice).toHaveTextContent("スキーマが新しいと、古いバージョンは起動できません");
  expect(screen.queryByTestId("version-pin-too-new")).toBeNull();
  expect(screen.queryByTestId("version-pin-unknown-schema")).toBeNull();
  expect(screen.getByTestId("version-pin-install-btn")).toBeEnabled();
});

test("ダウングレードは、バックアップに成功してからインストールを始める", async () => {
  const backup = deferred<string>();
  vi.mocked(api.backupBeforeInstall).mockReturnValue(backup.promise);
  const install = vi.fn(() => never<void>());
  const p = prepared({}, install);
  vi.mocked(api.prepareVersionInstall).mockResolvedValue(p);
  const { user } = await renderDialog();
  await choose(user, "1.1.0");

  await user.click(screen.getByTestId("version-pin-install-btn"));
  await settle();

  expect(screen.getByTestId("version-pin-backing-up")).toBeInTheDocument();
  expect(install).not.toHaveBeenCalled();

  backup.resolve("app_manual.db");
  await settle();

  expect(install).toHaveBeenCalledTimes(1);
  expect(screen.getByTestId("update-dialog-progress")).toBeInTheDocument();
});

test("バックアップに失敗したらインストールに進まず、理由を出して再試行できる", async () => {
  vi.mocked(api.backupBeforeInstall).mockRejectedValueOnce(new Error("ディスクが一杯"));
  const install = vi.fn(() => never<void>());
  vi.mocked(api.prepareVersionInstall).mockResolvedValue(prepared({}, install));
  const { user } = await renderDialog();
  await choose(user, "1.1.0");

  await user.click(screen.getByTestId("version-pin-install-btn"));
  await settle();

  expect(screen.getByTestId("version-pin-error")).toHaveTextContent("ディスクが一杯");
  expect(install).not.toHaveBeenCalled();

  await user.click(screen.getByTestId("version-pin-install-btn"));
  await settle();

  expect(api.backupBeforeInstall).toHaveBeenCalledTimes(2);
  expect(install).toHaveBeenCalledTimes(1);
});

test("バックアップが要らない(アップグレード)ときは、取らずにインストールを始める", async () => {
  const install = vi.fn(() => never<void>());
  vi.mocked(api.prepareVersionInstall).mockResolvedValue(
    prepared({ isDowngrade: false, backupRequired: false }, install),
  );
  const { user } = await renderDialog();
  await choose(user, "1.1.0");
  expect(screen.queryByTestId("version-pin-downgrade-notice")).toBeNull();

  await user.click(screen.getByTestId("version-pin-install-btn"));
  await settle();

  expect(api.backupBeforeInstall).not.toHaveBeenCalled();
  expect(install).toHaveBeenCalledTimes(1);
});

test("スキーマが新しすぎる版は赤い警告を出し、明示的に確認するまで進めない(バックアップは必須)", async () => {
  const install = vi.fn(() => never<void>());
  vi.mocked(api.prepareVersionInstall).mockResolvedValue(
    prepared({ schemaWarning: "tooNew", currentSchemaVersion: 13, targetSchemaVersion: 12 }, install),
  );
  const { user } = await renderDialog();
  await choose(user, "1.1.0");

  const warning = screen.getByTestId("version-pin-too-new");
  expect(warning).toHaveAttribute("role", "alert");
  expect(warning).toHaveTextContent("スキーマ版 13");
  expect(warning).toHaveTextContent("起動できなくなります");
  expect(screen.getByTestId("version-pin-install-btn")).toBeDisabled();

  await user.click(screen.getByTestId("version-pin-acknowledge"));
  expect(screen.getByTestId("version-pin-install-btn")).toBeEnabled();
  await user.click(screen.getByTestId("version-pin-install-btn"));
  await settle();

  expect(api.backupBeforeInstall).toHaveBeenCalledTimes(1);
  expect(install).toHaveBeenCalledTimes(1);
});

test("スキーマ版を判定できない版は、注意書きを出す", async () => {
  vi.mocked(api.prepareVersionInstall).mockResolvedValue(
    prepared({ schemaWarning: "unknown", targetSchemaVersion: null }),
  );
  const { user } = await renderDialog();
  await choose(user, "1.1.0");

  expect(screen.getByTestId("version-pin-unknown-schema")).toHaveTextContent("判定できません");
  expect(screen.queryByTestId("version-pin-too-new")).toBeNull();
  expect(screen.getByTestId("version-pin-install-btn")).toBeEnabled();
});

test("配布情報を取得できないと、一覧に戻ってエラーを出す", async () => {
  vi.mocked(api.prepareVersionInstall).mockRejectedValue(new Error("一覧にありません"));
  const { user } = await renderDialog();

  await choose(user, "1.1.0");

  expect(screen.getByTestId("version-pin-error")).toHaveTextContent("一覧にありません");
  expect(screen.getByTestId("version-pin-item-1.2.0")).toBeEnabled();
});

test("確認で戻ると、登録した更新を解放して一覧を取り直す", async () => {
  const p = prepared();
  vi.mocked(api.prepareVersionInstall).mockResolvedValue(p);
  const { user } = await renderDialog();
  await choose(user, "1.1.0");

  await user.click(screen.getByTestId("version-pin-back-btn"));
  await settle();

  expect(p.update.close).toHaveBeenCalled();
  expect(api.listInstallableReleases).toHaveBeenCalledTimes(2);
  expect(screen.getByTestId("version-pin-item-1.1.0")).toBeInTheDocument();
});

test("インストールに失敗したら理由を出し、再試行できる", async () => {
  const install = vi
    .fn<() => Promise<void>>()
    .mockRejectedValueOnce(new Error("署名が不正"))
    .mockImplementation(() => never<void>());
  vi.mocked(api.prepareVersionInstall).mockResolvedValue(prepared({}, install));
  const { user } = await renderDialog();
  await choose(user, "1.1.0");
  await user.click(screen.getByTestId("version-pin-install-btn"));
  await settle();

  expect(screen.getByTestId("version-pin-error")).toHaveTextContent("署名が不正");

  await user.click(screen.getByTestId("version-pin-retry-btn"));
  await settle();

  expect(install).toHaveBeenCalledTimes(2);
  expect(screen.getByTestId("update-dialog-progress")).toBeInTheDocument();
});

test("取得・バックアップ中は Escape で閉じない", async () => {
  vi.mocked(api.backupBeforeInstall).mockReturnValue(never<string>());
  vi.mocked(api.prepareVersionInstall).mockResolvedValue(prepared());
  const { user, onClose } = await renderDialog();
  await choose(user, "1.1.0");
  await user.click(screen.getByTestId("version-pin-install-btn"));
  await settle();

  await user.keyboard("{Escape}");

  expect(onClose).not.toHaveBeenCalled();
});
