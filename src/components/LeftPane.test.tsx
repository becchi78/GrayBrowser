// LeftPane の幅の制御: 一覧の下限幅の読み取り回数と、テーマ切り替え時の再クランプ。
import { act, fireEvent, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { renderWithProviders } from "../testing/render";
import { LeftPane } from "./LeftPane";

vi.mock("../api", () => ({
  getSidebarSectionsCollapsed: vi.fn(() => new Promise(() => {})),
  saveSidebarSectionsCollapsed: vi.fn(() => Promise.resolve()),
}));
vi.mock("./FolderSidebar", () => ({ FolderSidebar: () => null }));
vi.mock("./RatingBar", () => ({ RatingBar: () => null }));
vi.mock("./TagBar", () => ({ TagBar: () => null }));

// jsdom は CSS を読まないので、一覧の下限幅(CSS の --video-list-min-width)は
// 差し替え可能な値にし、読み取りの回数も数える。
const css = vi.hoisted(() => ({ minWidth: 554, navW: 200, reads: 0 }));
vi.mock("../lib/cssTokens", () => ({
  readLengthToken: (name: string) => {
    // 左ペインの初期幅(--nav-w)は、一覧の下限幅の読み取り回数には数えない。
    if (name === "--nav-w") return css.navW;
    css.reads += 1;
    return css.minWidth;
  },
}));

const originalInnerWidth = window.innerWidth;

function setWindowWidth(width: number) {
  Object.defineProperty(window, "innerWidth", { configurable: true, value: width });
}

beforeEach(() => {
  css.minWidth = 554;
  css.navW = 200;
  css.reads = 0;
  setWindowWidth(1400);
  document.documentElement.removeAttribute("data-theme");
});

afterEach(() => {
  setWindowWidth(originalInnerWidth);
});

const pane = () => screen.getByTestId("left-pane");

function drag(to: number, moves = 1) {
  fireEvent.mouseDown(screen.getByTestId("folder-sidebar-resize-handle"), { clientX: 200 });
  for (let i = 0; i < moves; i++) fireEvent.mouseMove(window, { clientX: to });
  fireEvent.mouseUp(window);
}

const renderPane = () =>
  renderWithProviders(
    <LeftPane
      folderPath={null}
      onFolderPathChange={() => {}}
      ratings={[]}
      onRatingsChange={() => {}}
      tagIds={[]}
      onTagIdsChange={() => {}}
      counts={null}
    />,
  );

test("ドラッグ中は mousemove のたびに CSS を読まず、ドラッグ開始時に1回だけ読む", () => {
  renderPane();
  const afterMount = css.reads;

  drag(400, 20);

  expect(css.reads - afterMount).toBe(1);
  expect(pane().style.width).toBe("400px");
});

test("テーマを切り替えると、下限幅を読み直して幅を再クランプする", async () => {
  renderPane();
  drag(500);
  expect(pane().style.width).toBe("500px");

  // 狭いウィンドウで、下限幅の大きいテーマ(ここではコンソール)に切り替える。
  setWindowWidth(900);
  css.minWidth = 550;
  act(() => {
    document.documentElement.setAttribute("data-theme", "console");
  });

  // 900 - 32(.app の padding) - 550 = 318
  await waitFor(() => expect(pane().style.width).toBe("318px"));
});

test("window の resize でも下限幅を読み直して再クランプする", () => {
  renderPane();
  drag(500);

  setWindowWidth(900);
  act(() => {
    window.dispatchEvent(new Event("resize"));
  });

  // 900 - 32 - 554 = 314
  expect(pane().style.width).toBe("314px");
});

test("初期幅は CSS の --nav-w から読む", () => {
  css.navW = 224;
  renderPane();
  expect(pane().style.width).toBe("224px");
});

test("--nav-w が下限・上限を外れていても、MIN_WIDTH〜MAX_WIDTH に収める", () => {
  css.navW = 120;
  const first = renderPane();
  expect(pane().style.width).toBe("200px");
  first.unmount();

  css.navW = 900;
  renderPane();
  expect(pane().style.width).toBe("500px");
});

test("ドラッグで変えた幅は、テーマを切り替えても --nav-w に戻らない", async () => {
  css.navW = 200;
  renderPane();
  drag(400);
  expect(pane().style.width).toBe("400px");

  const readsBefore = css.reads;
  css.navW = 224;
  act(() => {
    document.documentElement.setAttribute("data-theme", "graphite");
  });
  // テーマの切り替えで下限幅は読み直される(= 切り替えは処理された)が、幅は動かない。
  await waitFor(() => expect(css.reads).toBeGreaterThan(readsBefore));
  expect(pane().style.width).toBe("400px");
});
