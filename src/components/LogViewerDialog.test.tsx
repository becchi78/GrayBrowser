// LogViewerDialog の読み取り・絞り込み・追従を確かめる。
// 一覧は仮想化されており、jsdom では行の高さが 0 で行が描かれないため、
// 画面に出ている件数(「N / M 件」)で絞り込みの結果を確かめる。
// 読み取りは 1 秒ごとなので、フェイクタイマーで時間を進めて決定的に確かめる。
// フェイクタイマーの下では user-event の待ち合わせが進まないので、操作は fireEvent で行う。
import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import * as api from "../api";
import { deferred, settle } from "../testing/async";
import type { LogChunkDto, LogEntryDto } from "../types";
import { LogViewerDialog } from "./LogViewerDialog";

vi.mock("../api", () => ({
  readLog: vi.fn(),
  getLogDir: vi.fn(),
}));

const entry = (level: string, message: string, target = "gb::scan"): LogEntryDto => ({
  level,
  target,
  message,
});

const chunk = (
  entries: LogEntryDto[],
  next_offset = 100,
  reset = false,
): LogChunkDto => ({ entries, next_offset, reset });

const SAMPLE = [
  entry("ERROR", "壊れた"),
  entry("WARN", "あやしい"),
  entry("INFO", "Started scan", "gb::startup"),
  entry("DEBUG", "detail"),
  entry("UNKNOWN", "途中の断片"),
];

beforeEach(() => {
  vi.useFakeTimers();
  vi.spyOn(console, "error").mockImplementation(() => {});
  vi.mocked(api.readLog).mockResolvedValue(chunk(SAMPLE));
  vi.mocked(api.getLogDir).mockResolvedValue("C:\\logs");
});

afterEach(() => {
  vi.useRealTimers();
});

async function renderDialog() {
  const onClose = vi.fn();
  const view = render(<LogViewerDialog open={true} onClose={onClose} />);
  await settle();
  return { ...view, onClose };
}

async function advance(ms: number) {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(ms);
  });
}

const count = () => screen.getByTestId("log-viewer-count").textContent;
const level = (name: string) => screen.getByTestId(`log-viewer-level-${name}`);
const follow = () => screen.getByTestId("log-viewer-follow");
const search = () => screen.getByTestId("log-viewer-search");

test("開くと読み取った件数を出し、ログフォルダを出す", async () => {
  await renderDialog();

  expect(count()).toBe("5 / 5 件");
  expect(screen.getByTestId("log-viewer-dir")).toHaveTextContent("ログフォルダ: C:\\logs");
});

test("最初の読み取りは位置を指定せず、以降は前回の続きから 1 秒ごとに読む", async () => {
  vi.mocked(api.readLog)
    .mockResolvedValueOnce(chunk(SAMPLE, 100))
    .mockResolvedValueOnce(chunk([entry("INFO", "追加")], 140));
  await renderDialog();
  expect(api.readLog).toHaveBeenCalledTimes(1);
  expect(api.readLog).toHaveBeenLastCalledWith(null);

  await advance(999);
  expect(api.readLog).toHaveBeenCalledTimes(1);

  await advance(1);
  expect(api.readLog).toHaveBeenCalledTimes(2);
  expect(api.readLog).toHaveBeenLastCalledWith(100);
  expect(count()).toBe("6 / 6 件");
});

test("読み取りが長引いている間は、次の読み取りを重ねて始めない", async () => {
  const slow = deferred<LogChunkDto>();
  vi.mocked(api.readLog).mockReturnValueOnce(slow.promise);
  await renderDialog();

  await advance(5000);
  expect(api.readLog).toHaveBeenCalledTimes(1);

  await act(async () => slow.resolve(chunk(SAMPLE)));
  await advance(1000);
  expect(api.readLog).toHaveBeenCalledTimes(2);
});

test("reset を受け取ると、保持していた分を捨てて入れ替える", async () => {
  vi.mocked(api.readLog)
    .mockResolvedValueOnce(chunk(SAMPLE, 100))
    .mockResolvedValueOnce(chunk([entry("INFO", "新しいファイル")], 10, true));
  await renderDialog();

  await advance(1000);

  expect(count()).toBe("1 / 1 件");
});

test("読み取りに失敗するとエラーを出し続け、読めたら消す", async () => {
  vi.mocked(api.readLog)
    .mockResolvedValueOnce(chunk(SAMPLE))
    .mockRejectedValueOnce(new Error("ファイルを開けない"))
    .mockResolvedValueOnce(chunk([]));
  await renderDialog();
  expect(screen.queryByTestId("log-viewer-error")).toBeNull();

  await advance(1000);
  expect(screen.getByTestId("log-viewer-error")).toHaveTextContent("ファイルを開けない");
  expect(count()).toBe("5 / 5 件");

  await advance(1000);
  expect(screen.queryByTestId("log-viewer-error")).toBeNull();
});

test("閉じると読み取りを止める", async () => {
  const { unmount } = await renderDialog();
  expect(api.readLog).toHaveBeenCalledTimes(1);

  unmount();
  await advance(10000);

  expect(api.readLog).toHaveBeenCalledTimes(1);
});

test("閉じた後に遅れて届いた読み取りの結果で、次の読み取りを予約しない", async () => {
  const slow = deferred<LogChunkDto>();
  vi.mocked(api.readLog).mockReturnValueOnce(slow.promise);
  const { unmount } = await renderDialog();

  unmount();
  await act(async () => slow.resolve(chunk(SAMPLE)));
  await advance(10000);

  expect(api.readLog).toHaveBeenCalledTimes(1);
});

test("レベルのチェックを外すとそのレベルの行を隠し、戻すと出す。レベル不明の行は隠さない", async () => {
  await renderDialog();

  fireEvent.click(level("WARN"));
  expect(count()).toBe("4 / 5 件");

  fireEvent.click(level("ERROR"));
  fireEvent.click(level("INFO"));
  fireEvent.click(level("DEBUG"));
  expect(count()).toBe("1 / 5 件");

  fireEvent.click(level("WARN"));
  expect(count()).toBe("2 / 5 件");
});

test("検索語は、メッセージとモジュール名のどちらにも、大文字小文字を区別せず合う", async () => {
  await renderDialog();

  fireEvent.change(search(), { target: { value: "STARTED" } });
  expect(count()).toBe("1 / 5 件");

  fireEvent.change(search(), { target: { value: "gb::startup" } });
  expect(count()).toBe("1 / 5 件");

  fireEvent.change(search(), { target: { value: "存在しない" } });
  expect(count()).toBe("0 / 5 件");

  fireEvent.change(search(), { target: { value: "  " } });
  expect(count()).toBe("5 / 5 件");
});

test("レベルと検索語は組み合わせて絞り込む", async () => {
  await renderDialog();

  fireEvent.click(level("INFO"));
  fireEvent.change(search(), { target: { value: "gb::scan" } });

  expect(count()).toBe("4 / 5 件");
});

test("絞り込みの設定は、あとから届いた行にも当てはまる", async () => {
  vi.mocked(api.readLog)
    .mockResolvedValueOnce(chunk(SAMPLE))
    .mockResolvedValueOnce(chunk([entry("WARN", "もう一つ"), entry("ERROR", "もう一つ壊れた")], 200));
  await renderDialog();
  fireEvent.click(level("WARN"));

  await advance(1000);

  expect(count()).toBe("5 / 7 件");
});

/** スクロールの位置と大きさを、jsdom に代わってテストが決める。 */
function stubScroll({ scrollHeight, clientHeight }: { scrollHeight: number; clientHeight: number }) {
  const list = screen.getByTestId("log-viewer-list");
  let top = 0;
  Object.defineProperty(list, "scrollHeight", { configurable: true, value: scrollHeight });
  Object.defineProperty(list, "clientHeight", { configurable: true, value: clientHeight });
  Object.defineProperty(list, "scrollTop", {
    configurable: true,
    get: () => top,
    set: (v: number) => {
      top = v;
    },
  });
  return {
    list,
    /** ユーザーのスクロールを再現する(位置を決めてから scroll を発火)。 */
    scrollTo(v: number) {
      top = v;
      fireEvent.scroll(list);
    },
    top: () => top,
  };
}

test("追従は初期状態でオンで、行が増えるたびに末尾へ寄せる", async () => {
  vi.mocked(api.readLog)
    .mockResolvedValueOnce(chunk(SAMPLE))
    .mockResolvedValueOnce(chunk([entry("INFO", "追加")], 200));
  const view = await renderDialog();
  expect(follow()).toBeChecked();
  const s = stubScroll({ scrollHeight: 1000, clientHeight: 200 });
  expect(s.top()).toBe(0);

  await advance(1000);

  expect(s.top()).toBe(1000);
  view.unmount();
});

test("下端から離れる向きに縦スクロールすると、追従をやめ、行が増えても動かさない", async () => {
  vi.mocked(api.readLog)
    .mockResolvedValueOnce(chunk(SAMPLE))
    .mockResolvedValueOnce(chunk([entry("INFO", "追加")], 200));
  await renderDialog();
  const s = stubScroll({ scrollHeight: 1000, clientHeight: 200 });

  s.scrollTo(100);
  expect(follow()).not.toBeChecked();

  await advance(1000);
  expect(s.top()).toBe(100);
});

test("下端から 24px 以内へのスクロールでは追従をやめず、25px 離れたらやめる", async () => {
  await renderDialog();
  const s = stubScroll({ scrollHeight: 1000, clientHeight: 200 });

  s.scrollTo(776);
  expect(follow()).toBeChecked();

  s.scrollTo(775);
  expect(follow()).not.toBeChecked();
});

test("縦位置が動かない(横だけの)スクロールでは、下端から離れていても追従をやめない", async () => {
  await renderDialog();
  const s = stubScroll({ scrollHeight: 1000, clientHeight: 200 });

  s.scrollTo(0);

  expect(follow()).toBeChecked();
});

test("追従をオンに戻すと、末尾へ寄せる", async () => {
  await renderDialog();
  const s = stubScroll({ scrollHeight: 1000, clientHeight: 200 });
  s.scrollTo(100);
  expect(follow()).not.toBeChecked();

  fireEvent.click(follow());

  expect(follow()).toBeChecked();
  expect(s.top()).toBe(1000);
});

test("追従を切っている間のスクロール位置も覚えており、追従を戻した後に同じ位置で届くスクロールでは切れない", async () => {
  await renderDialog();
  const s = stubScroll({ scrollHeight: 1000, clientHeight: 200 });
  s.scrollTo(100);
  expect(follow()).not.toBeChecked();
  s.scrollTo(300);

  fireEvent.click(follow());
  expect(follow()).toBeChecked();
  // 追従で末尾へ寄せた後、縦位置が最後に見た位置と同じまま届くスクロール(横だけの動き)。
  s.scrollTo(300);

  expect(follow()).toBeChecked();
});
