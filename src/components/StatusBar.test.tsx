// StatusBar とジャンプメニューの現状の動作を固定する特性テスト。
import { beforeEach, expect, test, vi } from "vitest";
import { useEffect } from "react";
import { fireEvent, screen, waitFor } from "@testing-library/react";
import * as api from "../api";
import { UnreachableFoldersContext } from "../reachabilityContext";
import { renderWithProviders } from "../testing/render";
import { StatusBar } from "./StatusBar";

vi.mock("../api", () => ({
  getThumbnailProgress: vi.fn(),
}));

vi.mock("../events", () => ({
  onThumbnailProgress: vi.fn(() => () => {}),
}));

// 3つのパネルは、本物を描画すると一覧の取得・削除・確認ダイアログまで必要になる。
// ここで見たいのは StatusBar が持つ開閉の排他だけなので、件数をバッジへ報告し、
// 開いているときだけ目印を出す軽いダミーに差し替える。
function fakePanel(kind: string, count: number) {
  return function FakePanel({
    open,
    onCountChange,
  }: {
    open: boolean;
    onCountChange?: (count: number) => void;
  }) {
    useEffect(() => {
      onCountChange?.(count);
    }, [onCountChange]);
    return open ? <div data-testid={`panel-${kind}`} /> : null;
  };
}

vi.mock("./SkippedFilesPanel", () => ({ SkippedFilesPanel: fakePanel("unregistered", 3) }));
vi.mock("./GenerationFailuresPanel", () => ({
  GenerationFailuresPanel: fakePanel("failed", 2),
}));
vi.mock("./DuplicateGroupsPanel", () => ({ DuplicateGroupsPanel: fakePanel("duplicates", 4) }));

beforeEach(() => {
  vi.mocked(api.getThumbnailProgress).mockResolvedValue(null as never);
});

async function renderBar(props: { videoCount?: number; onJump?: (p: number) => void } = {}) {
  const onJump = props.onJump ?? vi.fn();
  const view = renderWithProviders(
    <StatusBar refreshKey={0} videoCount={props.videoCount ?? 10} selectedCount={0} onJump={onJump} />,
  );
  // パネルの件数がバッジに反映されるのを待つ(非同期の状態更新を落ち着かせる)。
  await screen.findByText("未登録 3");
  return { ...view, onJump };
}

const badge = (kind: "unregistered" | "duplicates" | "failed") =>
  screen.getByTestId(`status-badge-${kind}`);
const jumpButton = () => screen.getByTestId("status-jump-button");

test("パネルが報告した件数がバッジに出る", async () => {
  await renderBar();

  expect(screen.getByText("未登録 3")).toBeInTheDocument();
  expect(screen.getByText("重複 4")).toBeInTheDocument();
  expect(screen.getByText("生成失敗 2")).toBeInTheDocument();
});

test("バッジを押すと該当パネルが開き、もう一度押すと閉じる", async () => {
  const { user } = await renderBar();

  await user.click(badge("unregistered"));
  expect(screen.getByTestId("panel-unregistered")).toBeInTheDocument();
  expect(badge("unregistered")).toHaveAttribute("aria-pressed", "true");

  await user.click(badge("unregistered"));
  expect(screen.queryByTestId("panel-unregistered")).toBeNull();
  expect(badge("unregistered")).toHaveAttribute("aria-pressed", "false");
});

test("開けるパネルは同時に1つだけで、別のバッジを押すと切り替わる", async () => {
  const { user } = await renderBar();

  await user.click(badge("unregistered"));
  await user.click(badge("failed"));

  expect(screen.queryByTestId("panel-unregistered")).toBeNull();
  expect(screen.getByTestId("panel-failed")).toBeInTheDocument();
});

test("ジャンプメニューを開いたままバッジを押すと、メニューが閉じてパネルが開く", async () => {
  const { user } = await renderBar();
  await user.click(jumpButton());
  expect(screen.getByTestId("status-jump-menu")).toBeInTheDocument();

  // user.click は mousedown も送り、メニュー外の mousedown でも閉じてしまう。
  // バッジ側の排他だけを見るため、click だけを送る。
  fireEvent.click(badge("duplicates"));

  expect(screen.queryByTestId("status-jump-menu")).toBeNull();
  expect(screen.getByTestId("panel-duplicates")).toBeInTheDocument();
});

test("パネルを開いたままジャンプを押すと、パネルが閉じてメニューが開く", async () => {
  const { user } = await renderBar();
  await user.click(badge("failed"));
  expect(screen.getByTestId("panel-failed")).toBeInTheDocument();

  await user.click(jumpButton());

  expect(screen.queryByTestId("panel-failed")).toBeNull();
  expect(screen.getByTestId("status-jump-menu")).toBeInTheDocument();
});

test("ジャンプのボタンでメニューを開閉できる", async () => {
  const { user } = await renderBar();

  await user.click(jumpButton());
  expect(screen.getByTestId("status-jump-menu")).toBeInTheDocument();
  expect(jumpButton()).toHaveAttribute("aria-expanded", "true");

  await user.click(jumpButton());
  expect(screen.queryByTestId("status-jump-menu")).toBeNull();
  expect(jumpButton()).toHaveAttribute("aria-expanded", "false");
});

test("メニューの外で mousedown するとメニューが閉じる", async () => {
  const { user } = await renderBar();
  await user.click(jumpButton());

  // 外側のどこか(ここでは件数バッジ以外の body 直下)を押した状況を作る。
  fireEvent.mouseDown(document.body);

  expect(screen.queryByTestId("status-jump-menu")).toBeNull();
  await user.click(jumpButton());
  expect(screen.getByTestId("status-jump-menu")).toBeInTheDocument();
});

test("メニューの中で mousedown してもメニューは閉じない", async () => {
  const { user } = await renderBar();
  await user.click(jumpButton());

  fireEvent.mouseDown(screen.getByTestId("status-jump-menu"));

  expect(screen.getByTestId("status-jump-menu")).toBeInTheDocument();
});

test("Escape でメニューが閉じる", async () => {
  const { user } = await renderBar();
  await user.click(jumpButton());

  await user.keyboard("{Escape}");

  expect(screen.queryByTestId("status-jump-menu")).toBeNull();
});

test("パネルを開いているときの Escape ではパネルは閉じない", async () => {
  const { user } = await renderBar();
  await user.click(badge("unregistered"));

  await user.keyboard("{Escape}");

  // Escape の購読を持つのはジャンプメニューだけで、パネルの開閉には効かない。
  expect(screen.getByTestId("panel-unregistered")).toBeInTheDocument();
});

test("項目を選ぶとその割合で onJump が呼ばれ、メニューが閉じる", async () => {
  const { user, onJump } = await renderBar();
  await user.click(jumpButton());

  await user.click(screen.getByTestId("status-jump-item-50"));

  expect(onJump).toHaveBeenCalledTimes(1);
  expect(onJump).toHaveBeenCalledWith(50);
  expect(screen.queryByTestId("status-jump-menu")).toBeNull();
});

test("動画が0件のときジャンプのボタンは無効で、押してもメニューは開かない", async () => {
  const { user } = await renderBar({ videoCount: 0 });

  expect(jumpButton()).toBeDisabled();
  await user.click(jumpButton());

  expect(screen.queryByTestId("status-jump-menu")).toBeNull();
});

// rerender は根の要素を差し替えるので、初回描画と同じ Provider で包まないと
// 作り直されて状態が消え、「開き直さない」ことを確かめられない。
test("メニューを開いたまま動画が0件になると閉じ、件数が戻っても開き直さない", async () => {
  const { user, rerender, onJump } = await renderBar({ videoCount: 10 });
  await user.click(jumpButton());
  expect(screen.getByTestId("status-jump-menu")).toBeInTheDocument();

  rerender(
    <UnreachableFoldersContext.Provider value={[]}>
      <StatusBar refreshKey={0} videoCount={0} selectedCount={0} onJump={onJump} />
    </UnreachableFoldersContext.Provider>,
  );
  await waitFor(() => expect(screen.queryByTestId("status-jump-menu")).toBeNull());
  expect(jumpButton()).toBeDisabled();

  rerender(
    <UnreachableFoldersContext.Provider value={[]}>
      <StatusBar refreshKey={0} videoCount={10} selectedCount={0} onJump={onJump} />
    </UnreachableFoldersContext.Provider>,
  );

  expect(screen.queryByTestId("status-jump-menu")).toBeNull();
  expect(jumpButton()).toBeEnabled();
});
