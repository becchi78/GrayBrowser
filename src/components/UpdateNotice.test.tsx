// UpdateNotice の起動時の静かな更新確認を確かめる。
// 確認は起動から 5 秒待って行うため、フェイクタイマーで時間を進めて決定的に確かめる。
// フェイクタイマーの下では user-event の待ち合わせが進まないので、操作は fireEvent で行う。
import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import * as api from "../api";
import { deferred, never, settle } from "../testing/async";
import { fakeUpdate } from "../testing/fakes";
import { UpdateNotice } from "./UpdateNotice";

vi.mock("../api", () => ({
  checkForUpdate: vi.fn(),
  getUpdateCheckOnStartup: vi.fn(),
}));

beforeEach(() => {
  vi.useFakeTimers();
  vi.spyOn(console, "warn").mockImplementation(() => {});
  vi.mocked(api.getUpdateCheckOnStartup).mockResolvedValue(true);
  vi.mocked(api.checkForUpdate).mockResolvedValue(fakeUpdate(() => never()));
});

afterEach(() => {
  vi.useRealTimers();
});

async function advance(ms: number) {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(ms);
  });
}

const notice = () => screen.queryByTestId("update-notice");

test("起動から 5 秒経つまでは設定も更新も確認しない", async () => {
  render(<UpdateNotice onOpenDetails={vi.fn()} />);

  await advance(4999);

  expect(api.getUpdateCheckOnStartup).not.toHaveBeenCalled();
  expect(api.checkForUpdate).not.toHaveBeenCalled();
  expect(notice()).toBeNull();
});

test("5 秒経って更新があれば、新しいバージョンを知らせ、保持した更新を手放す", async () => {
  const update = fakeUpdate(() => never());
  vi.mocked(api.checkForUpdate).mockResolvedValue(update);
  render(<UpdateNotice onOpenDetails={vi.fn()} />);

  await advance(5000);

  expect(notice()).toHaveTextContent("新しいバージョン 2.0.0 があります");
  expect(update.close).toHaveBeenCalledTimes(1);
});

test("先行版の更新なら、通知に「先行版」と出す", async () => {
  vi.mocked(api.checkForUpdate).mockResolvedValue(fakeUpdate(() => never(), { channel: "preview" }));
  render(<UpdateNotice onOpenDetails={vi.fn()} />);

  await advance(5000);

  expect(notice()).toHaveTextContent("新しいバージョン 2.0.0 があります(先行版)");
});

test("安定版の更新なら、通知に「先行版」とは出さない", async () => {
  render(<UpdateNotice onOpenDetails={vi.fn()} />);

  await advance(5000);

  expect(notice()).not.toHaveTextContent("先行版");
});

test("起動時の確認を切る設定なら、更新を確認せず何も出さない", async () => {
  vi.mocked(api.getUpdateCheckOnStartup).mockResolvedValue(false);
  render(<UpdateNotice onOpenDetails={vi.fn()} />);

  await advance(5000);

  expect(api.checkForUpdate).not.toHaveBeenCalled();
  expect(notice()).toBeNull();
});

test("更新が無ければ何も出さない", async () => {
  vi.mocked(api.checkForUpdate).mockResolvedValue(null);
  render(<UpdateNotice onOpenDetails={vi.fn()} />);

  await advance(5000);

  expect(api.checkForUpdate).toHaveBeenCalledTimes(1);
  expect(notice()).toBeNull();
});

test("確認に失敗しても何も出さず、落ちない", async () => {
  vi.mocked(api.checkForUpdate).mockRejectedValue(new Error("オフライン"));
  render(<UpdateNotice onOpenDetails={vi.fn()} />);

  await advance(5000);

  expect(notice()).toBeNull();
  expect(console.warn).toHaveBeenCalled();
});

test("設定の読み取りに失敗しても何も出さず、落ちない", async () => {
  vi.mocked(api.getUpdateCheckOnStartup).mockRejectedValue(new Error("読めない"));
  render(<UpdateNotice onOpenDetails={vi.fn()} />);

  await advance(5000);

  expect(api.checkForUpdate).not.toHaveBeenCalled();
  expect(notice()).toBeNull();
});

test("待ち時間のうちに外れると、確認を始めない", async () => {
  const { unmount } = render(<UpdateNotice onOpenDetails={vi.fn()} />);
  await advance(1000);

  unmount();
  await advance(10000);

  expect(api.getUpdateCheckOnStartup).not.toHaveBeenCalled();
  expect(api.checkForUpdate).not.toHaveBeenCalled();
});

test("確認の応答を待つ間に外れても、受け取った更新は手放す", async () => {
  const pending = deferred<ReturnType<typeof fakeUpdate> | null>();
  vi.mocked(api.checkForUpdate).mockReturnValue(pending.promise);
  const { unmount } = render(<UpdateNotice onOpenDetails={vi.fn()} />);
  await advance(5000);
  unmount();

  const late = fakeUpdate(() => never());
  await act(async () => pending.resolve(late));
  await settle();

  expect(late.close).toHaveBeenCalledTimes(1);
});

test("「詳細・更新」は詳細を開く操作を届け、通知を消す", async () => {
  const onOpenDetails = vi.fn();
  render(<UpdateNotice onOpenDetails={onOpenDetails} />);
  await advance(5000);

  fireEvent.click(screen.getByTestId("update-notice-details-btn"));

  expect(onOpenDetails).toHaveBeenCalledTimes(1);
  expect(notice()).toBeNull();
});

test("「あとで」は詳細を開かずに通知を消し、再び出さない", async () => {
  const onOpenDetails = vi.fn();
  render(<UpdateNotice onOpenDetails={onOpenDetails} />);
  await advance(5000);

  fireEvent.click(screen.getByTestId("update-notice-later-btn"));
  await advance(60000);

  expect(onOpenDetails).not.toHaveBeenCalled();
  expect(notice()).toBeNull();
  expect(api.checkForUpdate).toHaveBeenCalledTimes(1);
});
