// VideoRow の現状の動作を固定する特性テスト。
import { beforeEach, expect, test, vi } from "vitest";
import { act, fireEvent, screen, within } from "@testing-library/react";
import * as api from "../api";
import { THUMBNAIL_CACHE_MAX_ENTRIES } from "../lib/boundedCache";
import { THUMBNAIL_PLACEHOLDER_LABELS } from "../lib/thumbnailPlaceholder";
import { UnreachableFoldersContext } from "../reachabilityContext";
import { renderWithProviders } from "../testing/render";
import type { VideoDto, VideoPropertiesDto } from "../types";
import { VideoRow } from "./VideoRow";

vi.mock("../api", () => ({
  getThumbnails: vi.fn(),
  getVideoProperties: vi.fn(),
  playVideo: vi.fn(),
  setRating: vi.fn(),
  assignTag: vi.fn(),
  listAllTags: vi.fn(),
  listTagsForVideo: vi.fn(),
  removeTag: vi.fn(),
}));

/** 外から解決・失敗させられる Promise。処理中の状態を作るために使う。 */
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

/** 解決しない Promise。処理が終わらない状態を保つために使う。 */
function never<T>(): Promise<T> {
  return new Promise<T>(() => {});
}

function video(overrides: Partial<VideoDto> = {}): VideoDto {
  return {
    id: "a",
    file_path: "C:\\videos\\a.mp4",
    file_name: "a.mp4",
    file_size: 5 * 1024 * 1024,
    duration: 125,
    quick_hash: "qh",
    full_hash: null,
    status: "online",
    rating: 0,
    created_at: "2026-01-01T00:00:00Z",
    thumbnail_ready: true,
    thumbnail_failed: false,
    ...overrides,
  };
}

function probed(overrides: Partial<VideoPropertiesDto> = {}): VideoPropertiesDto {
  return {
    width: 1920,
    height: 1080,
    video_codec: "h264",
    audio_codec: "aac",
    bitrate: 4_500_000,
    fps: 29.97,
    probed_at: "2026-01-01T00:00:00Z",
    ...overrides,
  };
}

function unprobed(): VideoPropertiesDto {
  return {
    width: null,
    height: null,
    video_codec: null,
    audio_codec: null,
    bitrate: null,
    fps: null,
    probed_at: null,
  };
}

const urlsOf = (prefix: string, n = 6) => Array.from({ length: n }, (_, i) => `${prefix}-${i}.webp`);

beforeEach(() => {
  // 失敗の経路では console.error に出すだけなので、テストの出力を汚さないよう黙らせる。
  vi.spyOn(console, "error").mockImplementation(() => {});
  vi.mocked(api.getThumbnails).mockResolvedValue(urlsOf("a"));
  vi.mocked(api.getVideoProperties).mockResolvedValue(probed());
  vi.mocked(api.playVideo).mockResolvedValue(undefined);
  vi.mocked(api.setRating).mockResolvedValue(undefined);
  vi.mocked(api.listTagsForVideo).mockResolvedValue([]);
  vi.mocked(api.listAllTags).mockResolvedValue([]);
});

/** 連なった Promise の解決と、それに続く再描画を落ち着かせる。 */
async function settle() {
  for (let i = 0; i < 3; i++) {
    await act(async () => {});
  }
}

interface RowProps {
  video: VideoDto;
  thumbnailVersion: number;
  catalogVersion: number;
  tagsVersion: number;
  isSelected: boolean;
  isMultiSelected: boolean;
}

async function renderRow({
  cache = new Map<string, string[]>(),
  unreachableFolders = [],
  ...overrides
}: Partial<RowProps> & {
  cache?: Map<string, string[]>;
  unreachableFolders?: readonly string[];
} = {}) {
  const onSelect = vi.fn();
  const onContextMenu = vi.fn();
  let current: RowProps = {
    video: video(),
    thumbnailVersion: 0,
    catalogVersion: 0,
    tagsVersion: 0,
    isSelected: false,
    isMultiSelected: false,
    ...overrides,
  };
  const ui = (p: RowProps) => (
    <VideoRow {...p} cache={cache} onSelect={onSelect} onContextMenu={onContextMenu} />
  );
  const view = renderWithProviders(ui(current), { unreachableFolders });
  // 初回描画の直後(取得が終わる前)の状態を確かめたいテストのために、settle の前に返す
  // 値も用意しておく。
  const firstImgs = view.container.querySelectorAll("img").length;
  await settle();
  // rerender は根の要素を差し替えるので、初回描画と同じ Provider で包まないと
  // 行が作り直されて状態が消え、同じ行での props の変化を確かめられない。
  const update = async (next: Partial<RowProps>) => {
    current = { ...current, ...next };
    view.rerender(
      <UnreachableFoldersContext.Provider value={unreachableFolders}>
        {ui(current)}
      </UnreachableFoldersContext.Provider>,
    );
    await settle();
  };
  return { ...view, cache, onSelect, onContextMenu, update, firstImgs };
}

const row = () => screen.getByTestId("video-row");
const thumbs = () => screen.getByTestId("video-row-thumbnails");
const slots = () => Array.from(thumbs().querySelectorAll<HTMLElement>(".video-row-thumbnail-wrap"));
const imgSrcs = () => Array.from(thumbs().querySelectorAll("img")).map((img) => img.getAttribute("src"));
const placeholders = () => Array.from(thumbs().querySelectorAll<HTMLElement>(".thumbnail-placeholder"));

/** プロパティ表を「見出し → 値」の組にして返す。 */
function propertyTable(): Record<string, string> {
  const dl = screen.getByTestId("video-row-properties");
  const dts = Array.from(dl.querySelectorAll("dt"));
  const dds = Array.from(dl.querySelectorAll("dd"));
  expect(dts).toHaveLength(dds.length);
  return Object.fromEntries(dts.map((dt, i) => [dt.textContent, dds[i].textContent]));
}

// ---- サムネイル ----

test("キャッシュにあれば初回描画から画像を出し、取得しない", async () => {
  const cache = new Map([["a:0", urlsOf("cached")]]);
  const { firstImgs } = await renderRow({ cache });

  expect(firstImgs).toBe(6);
  expect(imgSrcs()).toEqual(urlsOf("cached"));
  expect(api.getThumbnails).not.toHaveBeenCalled();
});

test("キャッシュのキーは id と thumbnailVersion の組で、版が違えば取得する", async () => {
  const cache = new Map([["a:0", urlsOf("old")]]);
  const { firstImgs } = await renderRow({ cache, thumbnailVersion: 1 });

  expect(firstImgs).toBe(0);
  expect(api.getThumbnails).toHaveBeenCalledTimes(1);
  expect(api.getThumbnails).toHaveBeenCalledWith("a");
  expect(imgSrcs()).toEqual(urlsOf("a"));
  expect(cache.get("a:1")).toEqual(urlsOf("a"));
  expect(cache.get("a:0")).toEqual(urlsOf("old"));
});

test("別の動画のキャッシュは使わない", async () => {
  const cache = new Map([["b:0", urlsOf("b")]]);
  await renderRow({ cache });

  expect(api.getThumbnails).toHaveBeenCalledWith("a");
  expect(imgSrcs()).toEqual(urlsOf("a"));
});

test("thumbnail_ready が false なら取得せず、6枠ともプレースホルダー", async () => {
  await renderRow({ video: video({ thumbnail_ready: false }) });

  expect(api.getThumbnails).not.toHaveBeenCalled();
  expect(imgSrcs()).toEqual([]);
  expect(placeholders()).toHaveLength(6);
});

test("取得したらキャッシュに入り、画像の alt はファイル名で、遅延読み込みにする", async () => {
  const { cache } = await renderRow();

  expect(api.getThumbnails).toHaveBeenCalledTimes(1);
  expect(cache.get("a:0")).toEqual(urlsOf("a"));
  expect(cache.size).toBe(1);
  const imgs = Array.from(thumbs().querySelectorAll("img"));
  expect(imgs).toHaveLength(6);
  for (const img of imgs) {
    expect(img).toHaveAttribute("alt", "a.mp4");
    expect(img).toHaveAttribute("loading", "lazy");
  }
});

test("キャッシュが上限を超えたら最も古いエントリが追い出される", async () => {
  const cache = new Map<string, string[]>();
  for (let i = 0; i < THUMBNAIL_CACHE_MAX_ENTRIES; i++) cache.set(`old${i}:0`, [`old${i}`]);
  await renderRow({ cache });

  expect(cache.size).toBe(THUMBNAIL_CACHE_MAX_ENTRIES);
  expect(cache.has("old0:0")).toBe(false);
  expect(cache.has("old1:0")).toBe(true);
  expect(cache.get("a:0")).toEqual(urlsOf("a"));
});

test("取得結果が null なら何も変わらない", async () => {
  vi.mocked(api.getThumbnails).mockResolvedValue(null);
  const { cache } = await renderRow();

  expect(api.getThumbnails).toHaveBeenCalledTimes(1);
  expect(cache.size).toBe(0);
  expect(imgSrcs()).toEqual([]);
  expect(placeholders()).toHaveLength(6);
});

test("thumbnailVersion が変わった後に届いた古い取得結果は捨てる", async () => {
  const first = deferred<string[] | null>();
  const second = deferred<string[] | null>();
  vi.mocked(api.getThumbnails).mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
  const { cache, update } = await renderRow();

  await update({ thumbnailVersion: 1 });
  expect(api.getThumbnails).toHaveBeenCalledTimes(2);

  await act(async () => first.resolve(urlsOf("stale")));
  await settle();
  expect(imgSrcs()).toEqual([]);
  expect(cache.size).toBe(0);

  await act(async () => second.resolve(urlsOf("fresh")));
  await settle();
  expect(imgSrcs()).toEqual(urlsOf("fresh"));
  expect([...cache.keys()]).toEqual(["a:1"]);
});

test("video.id が変わった後に届いた古い取得結果は捨てる", async () => {
  const first = deferred<string[] | null>();
  const second = deferred<string[] | null>();
  vi.mocked(api.getThumbnails).mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
  const { cache, update } = await renderRow();

  await update({ video: video({ id: "b", file_name: "b.mp4", file_path: "C:\\videos\\b.mp4" }) });
  expect(api.getThumbnails).toHaveBeenNthCalledWith(2, "b");

  await act(async () => first.resolve(urlsOf("stale")));
  await settle();
  expect(imgSrcs()).toEqual([]);
  expect(cache.size).toBe(0);

  await act(async () => second.resolve(urlsOf("b")));
  await settle();
  expect(imgSrcs()).toEqual(urlsOf("b"));
  expect([...cache.keys()]).toEqual(["b:0"]);
});

test("足りない枠は先頭から画像を詰め、残りを生成中のプレースホルダーにする", async () => {
  vi.mocked(api.getThumbnails).mockResolvedValue(urlsOf("a", 2));
  await renderRow();

  const s = slots();
  expect(s).toHaveLength(6);
  expect(s[0].querySelector("img")).toHaveAttribute("src", "a-0.webp");
  expect(s[1].querySelector("img")).toHaveAttribute("src", "a-1.webp");
  for (const slot of s.slice(2)) {
    expect(slot.querySelector("img")).toBeNull();
    const ph = slot.querySelector(".thumbnail-placeholder");
    expect(ph).toHaveClass("thumbnail-placeholder--generating");
    expect(ph).toHaveTextContent(THUMBNAIL_PLACEHOLDER_LABELS.generating);
  }
});

test("生成に失敗した動画のプレースホルダーは失敗の表示(オフラインより優先)", async () => {
  await renderRow({
    video: video({ thumbnail_ready: false, thumbnail_failed: true, status: "offline" }),
  });

  for (const ph of placeholders()) {
    expect(ph).toHaveClass("thumbnail-placeholder--failed");
    expect(ph.textContent).toBe(THUMBNAIL_PLACEHOLDER_LABELS.failed);
  }
});

test("オフラインの動画のプレースホルダーはオフラインの表示", async () => {
  await renderRow({ video: video({ thumbnail_ready: false, status: "offline" }) });

  expect(placeholders()).toHaveLength(6);
  for (const ph of placeholders()) {
    expect(ph).toHaveClass("thumbnail-placeholder--offline");
    expect(ph.textContent).toBe(THUMBNAIL_PLACEHOLDER_LABELS.offline);
  }
});

test("届かないフォルダ配下の動画のプレースホルダーもオフラインの表示", async () => {
  await renderRow({
    video: video({ thumbnail_ready: false, file_path: "N:\\nas\\a.mp4" }),
    unreachableFolders: ["n:/NAS"],
  });

  for (const ph of placeholders()) {
    expect(ph).toHaveClass("thumbnail-placeholder--offline");
  }
});

// ---- プロパティ ----

test("マウント時にプロパティを取得し、取得後は各項目を表示する", async () => {
  await renderRow();

  expect(api.getVideoProperties).toHaveBeenCalledTimes(1);
  expect(api.getVideoProperties).toHaveBeenCalledWith("a");
  expect(propertyTable()).toEqual({
    再生時間: "2:05",
    ファイルサイズ: "5.0 MB",
    解像度: "1920 × 1080",
    映像コーデック: "h264",
    音声コーデック: "aac",
    ビットレート: "4.5 Mbps",
    フレームレート: "29.97 fps",
  });
});

test("取得済みでも値が null の項目は「不明」と表示する", async () => {
  vi.mocked(api.getVideoProperties).mockResolvedValue({
    ...unprobed(),
    probed_at: "2026-01-01T00:00:00Z",
  });
  await renderRow({ video: video({ duration: null }) });

  expect(propertyTable()).toEqual({
    再生時間: "不明",
    ファイルサイズ: "5.0 MB",
    解像度: "不明",
    映像コーデック: "不明",
    音声コーデック: "不明",
    ビットレート: "不明",
    フレームレート: "不明",
  });
});

test("取得中は「詳細情報」「取得中...」を出し、再生時間とファイルサイズは出す", async () => {
  vi.mocked(api.getVideoProperties).mockReturnValue(never());
  await renderRow();

  expect(propertyTable()).toEqual({
    再生時間: "2:05",
    ファイルサイズ: "5.0 MB",
    詳細情報: "取得中...",
  });
});

test("probed_at が null の応答も取得中として扱う", async () => {
  vi.mocked(api.getVideoProperties).mockResolvedValue({ ...probed(), probed_at: null });
  await renderRow();

  expect(propertyTable()).toEqual({
    再生時間: "2:05",
    ファイルサイズ: "5.0 MB",
    詳細情報: "取得中...",
  });
});

test("応答が null の間は、catalogVersion が変わるたびに取り直す", async () => {
  vi.mocked(api.getVideoProperties).mockResolvedValue(null);
  const { update } = await renderRow();
  expect(api.getVideoProperties).toHaveBeenCalledTimes(1);

  await update({ catalogVersion: 1 });
  expect(api.getVideoProperties).toHaveBeenCalledTimes(2);

  vi.mocked(api.getVideoProperties).mockResolvedValue(probed());
  await update({ catalogVersion: 2 });
  expect(api.getVideoProperties).toHaveBeenCalledTimes(3);
  expect(propertyTable()["解像度"]).toBe("1920 × 1080");
});

test("未取得の応答が返っても、catalogVersion が変わらなければ取り直さない", async () => {
  vi.mocked(api.getVideoProperties).mockResolvedValue(unprobed());
  const { update } = await renderRow();
  await settle();
  expect(api.getVideoProperties).toHaveBeenCalledTimes(1);

  // 無関係な props の変化でも取り直さない。
  await update({ tagsVersion: 1 });
  expect(api.getVideoProperties).toHaveBeenCalledTimes(1);

  await update({ catalogVersion: 1 });
  expect(api.getVideoProperties).toHaveBeenCalledTimes(2);
  await settle();
  expect(api.getVideoProperties).toHaveBeenCalledTimes(2);
});

test("取得済みになった後は、catalogVersion が変わっても取り直さない", async () => {
  const { update } = await renderRow();
  expect(api.getVideoProperties).toHaveBeenCalledTimes(1);

  await update({ catalogVersion: 1 });
  await update({ catalogVersion: 2 });

  expect(api.getVideoProperties).toHaveBeenCalledTimes(1);
  expect(propertyTable()["解像度"]).toBe("1920 × 1080");
});

test("catalogVersion が変わった後に届いた古いプロパティは捨てる", async () => {
  const first = deferred<VideoPropertiesDto | null>();
  const second = deferred<VideoPropertiesDto | null>();
  vi.mocked(api.getVideoProperties)
    .mockReturnValueOnce(first.promise)
    .mockReturnValueOnce(second.promise);
  const { update } = await renderRow();

  await update({ catalogVersion: 1 });
  expect(api.getVideoProperties).toHaveBeenCalledTimes(2);

  await act(async () => first.resolve(probed({ video_codec: "stale" })));
  await settle();
  expect(propertyTable()["詳細情報"]).toBe("取得中...");

  await act(async () => second.resolve(probed({ video_codec: "fresh" })));
  await settle();
  expect(propertyTable()["映像コーデック"]).toBe("fresh");
});

// ---- サムネイル幅 ----

test("サムネイルの幅は高さ150に対する動画の縦横比から求め、6枠すべてに当てる", async () => {
  await renderRow();

  // round(150 * 1920 / 1080) = round(266.67) = 267
  expect(slots()).toHaveLength(6);
  for (const slot of slots()) {
    expect(slot.style.flexBasis).toBe("267px");
    expect(slot.style.width).toBe("267px");
  }
});

test("縦長の動画でも同じ式で幅を求める", async () => {
  vi.mocked(api.getVideoProperties).mockResolvedValue(probed({ width: 1080, height: 1920 }));
  await renderRow();

  // round(150 * 1080 / 1920) = round(84.375) = 84
  for (const slot of slots()) {
    expect(slot.style.flexBasis).toBe("84px");
    expect(slot.style.width).toBe("84px");
  }
});

test("取得前と、幅や高さが無いときの幅は200", async () => {
  vi.mocked(api.getVideoProperties).mockReturnValue(never());
  const { unmount } = await renderRow();
  for (const slot of slots()) {
    expect(slot.style.flexBasis).toBe("200px");
    expect(slot.style.width).toBe("200px");
  }
  unmount();

  vi.mocked(api.getVideoProperties).mockResolvedValue(probed({ width: 1920, height: null }));
  const second = await renderRow();
  for (const slot of slots()) {
    expect(slot.style.flexBasis).toBe("200px");
    expect(slot.style.width).toBe("200px");
  }
  second.unmount();

  vi.mocked(api.getVideoProperties).mockResolvedValue(probed({ width: null, height: 1080 }));
  await renderRow();
  for (const slot of slots()) {
    expect(slot.style.flexBasis).toBe("200px");
    expect(slot.style.width).toBe("200px");
  }
});

// ---- 再生 ----

test("サムネイル部分のダブルクリックで再生する", async () => {
  const { user } = await renderRow();

  await user.dblClick(thumbs());

  expect(api.playVideo).toHaveBeenCalledTimes(1);
  expect(api.playVideo).toHaveBeenCalledWith("C:\\videos\\a.mp4");
});

test("status が offline の行はダブルクリックしても再生しない", async () => {
  const { user } = await renderRow({ video: video({ status: "offline" }) });

  await user.dblClick(thumbs());

  expect(api.playVideo).not.toHaveBeenCalled();
});

test("届かないフォルダ配下の行はダブルクリックしても再生しない", async () => {
  const { user } = await renderRow({
    video: video({ file_path: "N:\\nas\\a.mp4" }),
    unreachableFolders: ["n:/NAS"],
  });

  await user.dblClick(thumbs());

  expect(api.playVideo).not.toHaveBeenCalled();
});

test("届かないフォルダの配下でなければ再生する", async () => {
  const { user } = await renderRow({ unreachableFolders: ["n:/NAS"] });

  await user.dblClick(thumbs());

  expect(api.playVideo).toHaveBeenCalledWith("C:\\videos\\a.mp4");
});

test("再生の開始中のダブルクリックは無視し、開始中の表示は出さない", async () => {
  const play = deferred<void>();
  vi.mocked(api.playVideo).mockReturnValueOnce(play.promise);
  const { user } = await renderRow();

  await user.dblClick(thumbs());
  expect(api.playVideo).toHaveBeenCalledTimes(1);
  expect(row().querySelector(".video-row-play-status")).toBeNull();
  expect(screen.queryByRole("alert")).toBeNull();

  await user.dblClick(thumbs());
  expect(api.playVideo).toHaveBeenCalledTimes(1);

  await act(async () => play.resolve());
  await settle();
  await user.dblClick(thumbs());
  expect(api.playVideo).toHaveBeenCalledTimes(2);
});

test("再生に失敗したら理由を alert で出し、次の再生の開始時に消す", async () => {
  vi.mocked(api.playVideo).mockRejectedValueOnce("プレーヤーがありません");
  const { user } = await renderRow();

  await user.dblClick(thumbs());
  await settle();
  expect(screen.getByRole("alert")).toHaveTextContent(/^再生に失敗しました: プレーヤーがありません$/);

  const play = deferred<void>();
  vi.mocked(api.playVideo).mockReturnValueOnce(play.promise);
  await user.dblClick(thumbs());
  expect(api.playVideo).toHaveBeenCalledTimes(2);
  expect(screen.queryByRole("alert")).toBeNull();

  await act(async () => play.resolve());
  await settle();
  expect(screen.queryByRole("alert")).toBeNull();
});

test("失敗の後に開始中のダブルクリックで失敗を重ねても、表示は1つ", async () => {
  vi.mocked(api.playVideo).mockRejectedValue("x");
  const { user } = await renderRow();

  await user.dblClick(thumbs());
  await settle();
  await user.dblClick(thumbs());
  await settle();

  expect(screen.getAllByRole("alert")).toHaveLength(1);
  expect(screen.getByRole("alert")).toHaveTextContent("再生に失敗しました: x");
});

test("ファイル名ラベルのダブルクリックでは再生しない", async () => {
  const { user } = await renderRow();

  await user.dblClick(screen.getByTestId("video-row-filename"));

  expect(api.playVideo).not.toHaveBeenCalled();
});

// ---- 選択・右クリック ----

test("サムネイル部分のクリックで修飾キーなしの選択を通知する", async () => {
  const { user, onSelect } = await renderRow();

  await user.click(thumbs());

  expect(onSelect).toHaveBeenCalledTimes(1);
  expect(onSelect).toHaveBeenCalledWith(
    expect.objectContaining({ id: "a" }),
    { ctrl: false, shift: false },
  );
});

test("クリックの修飾キーは ctrlKey/metaKey を ctrl、shiftKey を shift として渡す", async () => {
  const { onSelect } = await renderRow();

  fireEvent.click(thumbs(), { ctrlKey: true });
  fireEvent.click(thumbs(), { metaKey: true });
  fireEvent.click(thumbs(), { shiftKey: true });
  fireEvent.click(thumbs(), { ctrlKey: true, shiftKey: true });

  expect(onSelect.mock.calls.map((c) => c[1])).toEqual([
    { ctrl: true, shift: false },
    { ctrl: true, shift: false },
    { ctrl: false, shift: true },
    { ctrl: true, shift: true },
  ]);
});

test("ファイル名ラベルのクリックでも選択を通知し、修飾キーも同じ規則で渡す", async () => {
  const { user, onSelect } = await renderRow();
  const label = screen.getByTestId("video-row-filename");

  await user.click(label);
  fireEvent.click(label, { metaKey: true });
  fireEvent.click(label, { shiftKey: true });

  expect(onSelect).toHaveBeenCalledTimes(3);
  expect(onSelect.mock.calls[0][0]).toEqual(expect.objectContaining({ id: "a" }));
  expect(onSelect.mock.calls.map((c) => c[1])).toEqual([
    { ctrl: false, shift: false },
    { ctrl: true, shift: false },
    { ctrl: false, shift: true },
  ]);
});

test("情報の側(プロパティ表・星・タグ)をクリックしても選択しない", async () => {
  const { user, onSelect } = await renderRow();

  await user.click(screen.getByTestId("video-row-properties"));
  await user.click(screen.getByRole("radio", { name: "星2" }));
  await user.click(screen.getByPlaceholderText("タグを追加"));
  await user.click(screen.getByTestId("video-row-tags"));
  await settle();

  expect(onSelect).not.toHaveBeenCalled();
});

test("行の右クリックで動画とイベントを通知する", async () => {
  const { onContextMenu } = await renderRow();

  fireEvent.contextMenu(screen.getByTestId("video-row-properties"));

  expect(onContextMenu).toHaveBeenCalledTimes(1);
  const [v, e] = onContextMenu.mock.calls[0];
  expect(v).toEqual(expect.objectContaining({ id: "a" }));
  expect(e.type).toBe("contextmenu");
});

// ---- 見た目 ----

test("通常の行の className・data-testid・title", async () => {
  await renderRow();

  expect(row().className).toBe("video-row");
  expect(row()).toContainElement(screen.getByTestId("video-row-thumbnails-wrap"));
  expect(screen.getByTestId("video-row-thumbnails-wrap")).toContainElement(thumbs());
  expect(screen.getByTestId("video-row-thumbnails-wrap")).toContainElement(
    screen.getByTestId("video-row-filename"),
  );
  expect(thumbs()).toHaveAttribute("title", "a.mp4");
  expect(screen.getByTestId("video-row-filename")).toHaveAttribute("title", "a.mp4");
  expect(screen.getByTestId("video-row-filename")).toHaveTextContent(/^a\.mp4$/);
  expect(row()).toContainElement(screen.getByTestId("video-row-properties"));
  expect(screen.queryByText("⚠ オフライン")).toBeNull();
});

test("選択中・複数選択中の className", async () => {
  const { update } = await renderRow({ isSelected: true });
  expect(row()).toHaveClass("video-row", "video-row--selected");
  expect(row()).not.toHaveClass("video-row--multi-selected");

  await update({ isMultiSelected: true });
  expect(row()).toHaveClass("video-row", "video-row--selected", "video-row--multi-selected");

  await update({ isSelected: false, isMultiSelected: false });
  expect(row().className).toBe("video-row");
});

test("status が offline の行はオフラインの className とバッジを出す", async () => {
  await renderRow({ video: video({ status: "offline" }) });

  expect(row()).toHaveClass("video-row--offline");
  expect(within(screen.getByTestId("video-row-thumbnails-wrap")).getByText("⚠ オフライン")).toBeVisible();
});

test("届かないフォルダ配下の行もオフラインの className とバッジを出す", async () => {
  await renderRow({
    video: video({ file_path: "N:\\nas\\a.mp4" }),
    unreachableFolders: ["n:/NAS"],
  });

  expect(row()).toHaveClass("video-row--offline");
  expect(screen.getByText("⚠ オフライン")).toBeInTheDocument();
});

const checkedStar = () =>
  screen.getAllByRole("radio").filter((r) => r.getAttribute("aria-checked") === "true");

test("レーティングは video.rating から表示し、星のクリックで行内の表示が変わる", async () => {
  const { user } = await renderRow({ video: video({ rating: 3 }) });

  expect(checkedStar().map((r) => r.getAttribute("aria-label"))).toEqual(["星3"]);
  expect(screen.getByRole("radiogroup", { name: "レーティング" }).querySelectorAll(".star--filled")).toHaveLength(3);

  await user.click(screen.getByRole("radio", { name: "星5" }));
  await settle();

  expect(api.setRating).toHaveBeenCalledWith("a", 5);
  expect(checkedStar().map((r) => r.getAttribute("aria-label"))).toEqual(["星5"]);

  await user.click(screen.getByRole("button", { name: "レーティングをクリア" }));
  await settle();

  expect(api.setRating).toHaveBeenLastCalledWith("a", 0);
  expect(checkedStar()).toEqual([]);
  expect(screen.getByRole("button", { name: "レーティングをクリア" })).toBeDisabled();
});

test("タグ欄はこの動画のタグを取得して表示し、tagsVersion が変わると取り直す", async () => {
  vi.mocked(api.listTagsForVideo).mockResolvedValue([{ id: 1, name: "旅行" }]);
  const { update } = await renderRow();

  expect(api.listTagsForVideo).toHaveBeenCalledWith("a");
  expect(within(screen.getByTestId("video-row-tags")).getByText("旅行")).toBeInTheDocument();

  await update({ tagsVersion: 1 });
  expect(api.listTagsForVideo).toHaveBeenCalledTimes(2);
});
