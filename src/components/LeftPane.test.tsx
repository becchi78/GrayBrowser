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
const css = vi.hoisted(() => ({ minWidth: 532, reads: 0 }));
vi.mock("../lib/cssTokens", () => ({
  readLengthToken: () => {
    css.reads += 1;
    return css.minWidth;
  },
}));

const originalInnerWidth = window.innerWidth;

function setWindowWidth(width: number) {
  Object.defineProperty(window, "innerWidth", { configurable: true, value: width });
}

beforeEach(() => {
  css.minWidth = 532;
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

  // 狭いウィンドウで、下限幅の大きいテーマ(コンソール)に切り替える。
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

  // 900 - 32 - 532 = 336
  expect(pane().style.width).toBe("336px");
});
