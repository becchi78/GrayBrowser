// UpdateDialog の確認・案内・ダウンロード・失敗と再試行の状態の変化を確かめる。
// 状態遷移の規則そのものは lib/updateState のテストが持つので、ここでは
// 画面への出方と、確認結果の破棄・Update の解放・二重適用の防止を見る。
import { act, screen } from "@testing-library/react";
import { getVersion } from "@tauri-apps/api/app";
import { beforeEach, expect, test, vi } from "vitest";
import * as api from "../api";
import { deferred, never, settle } from "../testing/async";
import { fakeUpdate } from "../testing/fakes";
import { renderWithProviders } from "../testing/render";
import { UpdateDialog } from "./UpdateDialog";

vi.mock("../api", () => ({
  checkForUpdate: vi.fn(),
}));

type Update = NonNullable<Awaited<ReturnType<typeof api.checkForUpdate>>>;
type OnEvent = Parameters<Update["downloadAndInstall"]>[0] & {};

const MB = 1024 * 1024;

beforeEach(() => {
  vi.spyOn(console, "error").mockImplementation(() => {});
  vi.mocked(getVersion).mockResolvedValue("1.2.3");
});

function renderDialog(onClose = vi.fn()) {
  const view = renderWithProviders(<UpdateDialog open={true} onClose={onClose} />);
  return { ...view, onClose };
}

/** ダウンロードの進捗を、テストの好きな時点で流せる更新。 */
function controllableUpdate() {
  const finish = deferred<void>();
  let onEvent: OnEvent | null = null;
  const update = fakeUpdate((cb) => {
    onEvent = cb;
    return finish.promise;
  });
  return {
    update,
    finish,
    emit: (event: Parameters<OnEvent>[0]) =>
      act(() => {
        onEvent?.(event);
      }),
  };
}

const text = (id: string) => screen.getByTestId(id);

test("確認中は案内を出し、確認が終わるまで主ボタンを出さない", async () => {
  vi.mocked(api.checkForUpdate).mockReturnValue(never());

  renderDialog();
  await settle();

  expect(text("update-dialog-checking")).toBeInTheDocument();
  expect(screen.queryByTestId("update-dialog-apply-btn")).toBeNull();
});

test("更新が無いと「最新です」と現在のバージョンを出す", async () => {
  vi.mocked(api.checkForUpdate).mockResolvedValue(null);

  renderDialog();

  expect(await screen.findByTestId("update-dialog-up-to-date")).toHaveTextContent(
    "最新です(現在のバージョン 1.2.3)",
  );
});

test("更新が無く、現在のバージョンを取れなくても「最新です」だけを出す", async () => {
  vi.mocked(api.checkForUpdate).mockResolvedValue(null);
  vi.mocked(getVersion).mockRejectedValue(new Error("取れない"));

  renderDialog();

  const message = await screen.findByTestId("update-dialog-up-to-date");
  expect(message).toHaveTextContent(/^最新です$/);
});

test("更新があると新しいバージョンと現在のバージョン、リリースノートを出す", async () => {
  vi.mocked(api.checkForUpdate).mockResolvedValue(fakeUpdate(() => never()));

  renderDialog();

  expect(await screen.findByTestId("update-dialog-available")).toHaveTextContent(
    "新しいバージョン 2.0.0 があります(現在のバージョン 1.0.0)",
  );
  expect(text("update-dialog-notes")).toHaveTextContent("ノート");
  expect(text("update-dialog-apply-btn")).toHaveFocus();
});

test("提示された版が先行配信のものなら、チャネル名と公開前であることを目立つ形で明示する", async () => {
  vi.mocked(api.checkForUpdate).mockResolvedValue(
    fakeUpdate(() => never(), { channel: "preview" }),
  );

  renderDialog();

  expect(await screen.findByTestId("update-dialog-dev-notice")).toHaveTextContent(
    "公開前の確認中のビルドです。問題がなければ、このビルドが安定版として公開されます。動作確認用のため、不安定な場合があります。",
  );
  expect(screen.getByTestId("update-dialog-channel")).toHaveTextContent("先行版");
});

test("提示された版が安定版なら、先行版の表示は出さない", async () => {
  vi.mocked(api.checkForUpdate).mockResolvedValue(fakeUpdate(() => never()));

  renderDialog();

  await screen.findByTestId("update-dialog-available");
  expect(screen.queryByTestId("update-dialog-dev-notice")).toBeNull();
  expect(screen.getByTestId("update-dialog-channel")).toHaveTextContent("安定版");
});

test.each([
  { name: "空文字", body: "" },
  { name: "空白だけ", body: "  \n " },
  { name: "null", body: null },
])("リリースノートが$nameなら、ノートの欄を出さない", async ({ body }) => {
  vi.mocked(api.checkForUpdate).mockResolvedValue(fakeUpdate(() => never(), { body }));

  renderDialog();

  await screen.findByTestId("update-dialog-available");
  expect(screen.queryByTestId("update-dialog-notes")).toBeNull();
});

test("「あとで」で閉じる操作が届く", async () => {
  vi.mocked(api.checkForUpdate).mockResolvedValue(fakeUpdate(() => never()));
  const { user, onClose } = renderDialog();

  await user.click(await screen.findByTestId("update-dialog-later-btn"));

  expect(onClose).toHaveBeenCalledTimes(1);
});

test("確認に失敗すると理由を出し、再試行で確認をやり直して結果に進む", async () => {
  vi.mocked(api.checkForUpdate).mockRejectedValueOnce(new Error("接続できない"));
  vi.mocked(api.checkForUpdate).mockResolvedValueOnce(fakeUpdate(() => never()));
  const { user } = renderDialog();

  const error = await screen.findByTestId("update-dialog-error");
  expect(error).toHaveTextContent("接続できない");
  expect(screen.getByText("更新を確認できませんでした。")).toBeInTheDocument();

  await user.click(text("update-dialog-retry-btn"));

  expect(await screen.findByTestId("update-dialog-available")).toBeInTheDocument();
  expect(api.checkForUpdate).toHaveBeenCalledTimes(2);
  expect(screen.queryByTestId("update-dialog-error")).toBeNull();
});

test("「今すぐ更新」でダウンロードが始まり、進捗が割合と受信量で出る", async () => {
  const { update, emit } = controllableUpdate();
  vi.mocked(api.checkForUpdate).mockResolvedValue(update);
  const { user } = renderDialog();

  await user.click(await screen.findByTestId("update-dialog-apply-btn"));

  expect(text("update-dialog-progress-text")).toHaveTextContent("0 KB");
  await emit({ event: "Started", data: { contentLength: 4 * MB } });
  expect(text("update-dialog-progress-text")).toHaveTextContent("0%(0 KB / 4.0 MB)");
  await emit({ event: "Progress", data: { chunkLength: MB } });
  await emit({ event: "Progress", data: { chunkLength: MB } });
  expect(text("update-dialog-progress-text")).toHaveTextContent("50%(2.0 MB / 4.0 MB)");
  expect(text("update-dialog-progress")).toHaveAttribute("value", "50");
  expect(screen.queryByTestId("update-dialog-apply-btn")).toBeNull();
  expect(screen.queryByTestId("update-dialog-close-btn")).toBeNull();
});

test("全体の大きさが分からないダウンロードは、割合を出さず受信量だけを出す", async () => {
  const { update, emit } = controllableUpdate();
  vi.mocked(api.checkForUpdate).mockResolvedValue(update);
  const { user } = renderDialog();
  await user.click(await screen.findByTestId("update-dialog-apply-btn"));

  await emit({ event: "Started", data: {} });
  await emit({ event: "Progress", data: { chunkLength: 3 * MB } });

  expect(text("update-dialog-progress-text")).toHaveTextContent(/^3\.0 MB$/);
  expect(text("update-dialog-progress")).not.toHaveAttribute("value");
});

test("ダウンロードが終わると、インストーラーの起動待ちの案内に変わる", async () => {
  const { update, emit } = controllableUpdate();
  vi.mocked(api.checkForUpdate).mockResolvedValue(update);
  const { user } = renderDialog();
  await user.click(await screen.findByTestId("update-dialog-apply-btn"));
  await emit({ event: "Started", data: { contentLength: MB } });

  await emit({ event: "Finished" });

  expect(text("update-dialog-installing")).toBeInTheDocument();
  expect(screen.queryByTestId("update-dialog-progress")).toBeNull();
});

test("ダウンロードに失敗すると理由を出し、再試行で同じ更新のダウンロードをやり直す", async () => {
  const update = fakeUpdate(() => never());
  vi.mocked(update.downloadAndInstall).mockRejectedValueOnce(new Error("通信が切れた"));
  vi.mocked(api.checkForUpdate).mockResolvedValue(update);
  const { user } = renderDialog();

  await user.click(await screen.findByTestId("update-dialog-apply-btn"));

  expect(await screen.findByTestId("update-dialog-error")).toHaveTextContent("通信が切れた");
  expect(screen.getByText("更新に失敗しました。")).toBeInTheDocument();

  await user.click(text("update-dialog-retry-btn"));

  expect(update.downloadAndInstall).toHaveBeenCalledTimes(2);
  expect(api.checkForUpdate).toHaveBeenCalledTimes(1);
  expect(text("update-dialog-progress")).toBeInTheDocument();
});

test("適用の開始から再描画までの間に重ねて押されても、ダウンロードは1回しか始まらない", async () => {
  const update = fakeUpdate(() => never());
  vi.mocked(api.checkForUpdate).mockResolvedValue(update);
  renderDialog();
  const button = await screen.findByTestId("update-dialog-apply-btn");

  // act の中では再描画が保留されるので、ボタンが消える前の連打を再現できる。
  act(() => {
    button.click();
    button.click();
  });

  expect(update.downloadAndInstall).toHaveBeenCalledTimes(1);
});

test("閉じた後に遅れて届いた確認結果は表示に使わず、受け取った更新を手放す", async () => {
  const pending = deferred<Update | null>();
  vi.mocked(api.checkForUpdate).mockReturnValue(pending.promise);
  const { unmount } = renderDialog();
  await settle();
  unmount();

  const late = fakeUpdate(() => never());
  await act(async () => pending.resolve(late));
  await settle();

  expect(late.close).toHaveBeenCalledTimes(1);
});

test("閉じると、保持していた更新を手放す", async () => {
  const update = fakeUpdate(() => never());
  vi.mocked(api.checkForUpdate).mockResolvedValue(update);
  const { unmount } = renderDialog();
  await screen.findByTestId("update-dialog-available");
  expect(update.close).not.toHaveBeenCalled();

  unmount();

  expect(update.close).toHaveBeenCalledTimes(1);
});

test("open が false なら何も描かず、確認も始めない", async () => {
  renderWithProviders(<UpdateDialog open={false} onClose={vi.fn()} />);
  await settle();

  expect(screen.queryByTestId("update-dialog")).toBeNull();
  expect(api.checkForUpdate).not.toHaveBeenCalled();
});
