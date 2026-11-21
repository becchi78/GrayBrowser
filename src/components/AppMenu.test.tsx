// AppMenu: ハンバーガーの1階層目とサブメニューの内容・開閉・キーボード操作・位置。
import { fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import * as api from "../api";
import type { Appearance, Mode, Theme } from "../lib/appearance";
import manifest from "../themes/manifest.json";
import { registerUndismissibleModal } from "../undismissibleModals";
import { useActiveDialog, type DialogKind } from "../useActiveDialog";
import { AppMenu } from "./AppMenu";

vi.mock("../api", () => ({
  runManualBackup: vi.fn(),
  quitApp: vi.fn(),
}));

const DEFAULT_APPEARANCE: Appearance = { theme: "graphite", mode: "system" };

interface Handlers {
  openDialog: ReturnType<typeof vi.fn<(kind: DialogKind) => void>>;
  onSelectTheme: ReturnType<typeof vi.fn<(theme: Theme) => void>>;
  onSelectMode: ReturnType<typeof vi.fn<(mode: Mode) => void>>;
}

function renderMenu(appearance: Appearance = DEFAULT_APPEARANCE): Handlers {
  const handlers: Handlers = {
    openDialog: vi.fn<(kind: DialogKind) => void>(),
    onSelectTheme: vi.fn<(theme: Theme) => void>(),
    onSelectMode: vi.fn<(mode: Mode) => void>(),
  };
  render(<AppMenu appearance={appearance} {...handlers} />);
  return handlers;
}

const button = () => screen.getByRole("button", { name: "メニュー" });
const topMenu = () => screen.getByTestId("app-menu");
const queryTopMenu = () => screen.queryByTestId("app-menu");
const group = (id: string) => screen.getByTestId(`app-menu-group-${id}`);
const submenu = (id: string) => screen.getByTestId(`app-submenu-${id}`);
const querySubmenu = (id: string) => screen.queryByTestId(`app-submenu-${id}`);

async function openMenu() {
  await userEvent.click(button());
}

async function openGroup(id: string) {
  await openMenu();
  await userEvent.click(group(id));
}

function labelsOf(el: HTMLElement): string[] {
  return Array.from(el.querySelectorAll('[role^="menuitem"]')).map((i) => i.textContent ?? "");
}

const SUBMENUS: Record<string, string[]> = {
  file: ["フォルダ管理...", ".wbインポート...", "データベースをバックアップ", "終了"],
  tag: ["タグの編集..."],
  theme: ["Graphite", "Navy Console", "Fluent Mica", "Dark Cinema", "System", "Light", "Dark"],
  settings: ["設定..."],
  help: ["動作ログ...", "更新の確認...", "バージョン情報"],
};

beforeEach(() => {
  vi.spyOn(console, "error").mockImplementation(() => {});
  vi.mocked(api.runManualBackup).mockResolvedValue(undefined);
  vi.mocked(api.quitApp).mockResolvedValue(undefined);
});

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

// ---- 1階層目 ----

test("閉じている間は、ボタンだけがあり、メニューは無い", () => {
  renderMenu();
  expect(button()).toHaveAttribute("aria-haspopup", "menu");
  expect(button()).toHaveAttribute("aria-expanded", "false");
  expect(queryTopMenu()).toBeNull();
});

test("ボタンを押すと開き、aria-expanded が true になる", async () => {
  renderMenu();
  await openMenu();
  expect(screen.getByRole("menu", { name: "メニュー" })).toBeInTheDocument();
  expect(button()).toHaveAttribute("aria-expanded", "true");
});

test("1階層目は5つのグループだけが並び、葉は直接並ばない", async () => {
  renderMenu();
  await openMenu();
  expect(labelsOf(topMenu())).toEqual(["ファイル", "タグ", "テーマ", "設定", "ヘルプ"]);
  expect(screen.queryByRole("menuitemradio")).toBeNull();
});

test("1階層目の項目は aria-haspopup=menu を持ち、閉じている間は aria-expanded が false", async () => {
  renderMenu();
  await openMenu();
  for (const id of Object.keys(SUBMENUS)) {
    expect(group(id)).toHaveAttribute("role", "menuitem");
    expect(group(id)).toHaveAttribute("aria-haspopup", "menu");
    expect(group(id)).toHaveAttribute("aria-expanded", "false");
  }
});

test("Enter でも Space でもボタンから開ける", async () => {
  renderMenu();
  button().focus();
  await userEvent.keyboard("{Enter}");
  expect(queryTopMenu()).toBeInTheDocument();

  await userEvent.keyboard("{Escape}");
  expect(queryTopMenu()).toBeNull();

  await userEvent.keyboard(" ");
  expect(queryTopMenu()).toBeInTheDocument();
});

test("開くと1階層目の最初の項目にフォーカスがある", async () => {
  renderMenu();
  await openMenu();
  expect(group("file")).toHaveFocus();
});

// ---- サブメニューの内容 ----

test.each(Object.entries(SUBMENUS))("「%s」を開くと、その内容が順に並ぶ", async (id, labels) => {
  renderMenu();
  await openGroup(id);
  expect(labelsOf(submenu(id))).toEqual(labels);
  expect(group(id)).toHaveAttribute("aria-expanded", "true");
  expect(group(id)).toHaveAttribute("aria-controls", submenu(id).id);
});

test("サブメニューは role=menu で、グループ名の aria-label を持ち、親の項目と同じ階層に入る", async () => {
  renderMenu();
  await openGroup("file");
  const sub = screen.getByRole("menu", { name: "ファイル" });
  expect(sub).toBe(submenu("file"));
  expect(topMenu()).toContainElement(sub);
  expect(sub.parentElement).toContainElement(group("file"));
});

test("テーマは、テーマ4種と外観3種の間に区切りがある", async () => {
  renderMenu();
  await openGroup("theme");
  const children = Array.from(submenu("theme").children).map(
    (el) => el.getAttribute("role") ?? "",
  );
  expect(children).toEqual([
    ...Array<string>(4).fill("menuitemradio"),
    "separator",
    ...Array<string>(3).fill("menuitemradio"),
  ]);
});

test("テーマのテーマ項目の id は、マニフェストのテーマ名と順序まで一致する", async () => {
  renderMenu();
  await openGroup("theme");
  const ids = Array.from(submenu("theme").querySelectorAll('[data-testid^="app-menu-item-theme-"]')).map(
    (el) => (el.getAttribute("data-testid") ?? "").replace("app-menu-item-theme-", ""),
  );
  expect(ids).toEqual(manifest.themes);
});

test("同時に開くサブメニューは1つだけ", async () => {
  renderMenu();
  await openGroup("file");
  await userEvent.click(group("help"));
  expect(querySubmenu("file")).toBeNull();
  expect(submenu("help")).toBeInTheDocument();
  expect(group("file")).toHaveAttribute("aria-expanded", "false");
});

// ---- ホバー ----

test("1階層目の項目にホバーするとサブメニューが開き、別の項目へ移ると切り替わる", async () => {
  renderMenu();
  await openMenu();

  await userEvent.hover(group("tag"));
  expect(submenu("tag")).toBeInTheDocument();
  expect(group("tag")).toHaveFocus();

  await userEvent.hover(group("theme"));
  expect(querySubmenu("tag")).toBeNull();
  expect(submenu("theme")).toBeInTheDocument();
});

test("サブメニューの外へマウスが出ても、サブメニューは閉じない", async () => {
  renderMenu();
  await openMenu();
  await userEvent.hover(group("file"));

  await userEvent.unhover(group("file"));
  fireEvent.mouseLeave(topMenu());

  expect(submenu("file")).toBeInTheDocument();
});

test("サブメニューの項目にホバーすると、その項目にフォーカスが移る", async () => {
  renderMenu();
  await openMenu();
  await userEvent.hover(group("file"));

  await userEvent.hover(screen.getByRole("menuitem", { name: "終了" }));

  expect(screen.getByRole("menuitem", { name: "終了" })).toHaveFocus();
});

// ---- 葉の選択 ----

test.each<[string, string, DialogKind]>([
  ["file", "フォルダ管理...", "folder"],
  ["file", ".wbインポート...", "wbImport"],
  ["tag", "タグの編集...", "tagBar"],
  ["settings", "設定...", "settings"],
  ["help", "動作ログ...", "log"],
  ["help", "更新の確認...", "update"],
  ["help", "バージョン情報", "about"],
])("%s の「%s」は %s のダイアログを開き、メニュー全体を閉じる", async (g, label, kind) => {
  const { openDialog } = renderMenu();
  await openGroup(g);

  await userEvent.click(screen.getByRole("menuitem", { name: label }));

  expect(openDialog).toHaveBeenCalledTimes(1);
  expect(openDialog).toHaveBeenCalledWith(kind);
  expect(queryTopMenu()).toBeNull();
  expect(button()).toHaveFocus();
});

test("「データベースをバックアップ」はバックアップのコマンドを呼ぶ", async () => {
  const { openDialog } = renderMenu();
  await openGroup("file");

  await userEvent.click(screen.getByRole("menuitem", { name: "データベースをバックアップ" }));

  expect(api.runManualBackup).toHaveBeenCalledTimes(1);
  expect(api.quitApp).not.toHaveBeenCalled();
  expect(openDialog).not.toHaveBeenCalled();
  expect(queryTopMenu()).toBeNull();
});

test("「終了」は終了のコマンドを呼ぶ", async () => {
  renderMenu();
  await openGroup("file");

  await userEvent.click(screen.getByRole("menuitem", { name: "終了" }));

  expect(api.quitApp).toHaveBeenCalledTimes(1);
  expect(api.runManualBackup).not.toHaveBeenCalled();
});

test("コマンドが失敗しても例外にせず、記録に残す", async () => {
  vi.mocked(api.runManualBackup).mockRejectedValue(new Error("x"));
  vi.mocked(api.quitApp).mockRejectedValue(new Error("y"));
  renderMenu();

  await openGroup("file");
  await userEvent.click(screen.getByRole("menuitem", { name: "データベースをバックアップ" }));
  await openGroup("file");
  await userEvent.click(screen.getByRole("menuitem", { name: "終了" }));

  expect(console.error).toHaveBeenCalledTimes(2);
});

test("現在のテーマと外観だけが aria-checked になる", async () => {
  renderMenu({ theme: "cinema", mode: "light" });
  await openGroup("theme");

  const radios = within(submenu("theme")).getAllByRole("menuitemradio");
  expect(radios).toHaveLength(7);
  expect(
    radios.filter((el) => el.getAttribute("aria-checked") === "true").map((el) => el.textContent),
  ).toEqual(["Dark Cinema", "Light"]);
});

test("テーマの項目を選ぶと選択関数を呼び、外観は呼ばずにメニューを閉じる", async () => {
  const { onSelectTheme, onSelectMode } = renderMenu();
  await openGroup("theme");

  await userEvent.click(screen.getByRole("menuitemradio", { name: "Fluent Mica" }));

  expect(onSelectTheme).toHaveBeenCalledWith("fluent");
  expect(onSelectMode).not.toHaveBeenCalled();
  expect(queryTopMenu()).toBeNull();
  expect(button()).toHaveFocus();
});

test("外観の項目を選ぶと選択関数を呼び、テーマは呼ばない", async () => {
  const { onSelectTheme, onSelectMode } = renderMenu();
  await openGroup("theme");

  await userEvent.click(screen.getByRole("menuitemradio", { name: "Dark" }));

  expect(onSelectMode).toHaveBeenCalledWith("dark");
  expect(onSelectTheme).not.toHaveBeenCalled();
});

// ---- キーボード ----

test("1階層目で↓↑は循環し、Home/End で先頭/末尾へ移る", async () => {
  renderMenu();
  await openMenu();

  await userEvent.keyboard("{ArrowDown}");
  expect(group("tag")).toHaveFocus();

  await userEvent.keyboard("{ArrowUp}{ArrowUp}");
  expect(group("help")).toHaveFocus();

  await userEvent.keyboard("{ArrowDown}");
  expect(group("file")).toHaveFocus();

  await userEvent.keyboard("{End}");
  expect(group("help")).toHaveFocus();
  await userEvent.keyboard("{Home}");
  expect(group("file")).toHaveFocus();
});

test("1階層目の↓↑は、開いているサブメニューの項目を巻き込まない", async () => {
  renderMenu();
  await openGroup("file");
  group("file").focus();

  await userEvent.keyboard("{ArrowDown}");

  expect(group("tag")).toHaveFocus();
});

test("→ でサブメニューが開き、最初の項目にフォーカスが移る", async () => {
  renderMenu();
  await openMenu();

  await userEvent.keyboard("{ArrowRight}");

  expect(submenu("file")).toBeInTheDocument();
  expect(screen.getByRole("menuitem", { name: "フォルダ管理..." })).toHaveFocus();
});

test("Enter でも Space でも1階層目の項目からサブメニューを開ける", async () => {
  renderMenu();
  await openMenu();

  await userEvent.keyboard("{Enter}");
  expect(screen.getByRole("menuitem", { name: "フォルダ管理..." })).toHaveFocus();

  await userEvent.keyboard("{ArrowLeft}{ArrowDown}");
  expect(group("tag")).toHaveFocus();
  await userEvent.keyboard(" ");
  expect(screen.getByRole("menuitem", { name: "タグの編集..." })).toHaveFocus();
});

test("サブメニュー内で↓↑は循環し、Home/End で先頭/末尾へ移る", async () => {
  renderMenu();
  await openMenu();
  await userEvent.keyboard("{ArrowRight}");

  await userEvent.keyboard("{ArrowUp}");
  expect(screen.getByRole("menuitem", { name: "終了" })).toHaveFocus();
  await userEvent.keyboard("{ArrowDown}");
  expect(screen.getByRole("menuitem", { name: "フォルダ管理..." })).toHaveFocus();

  await userEvent.keyboard("{End}");
  expect(screen.getByRole("menuitem", { name: "終了" })).toHaveFocus();
  await userEvent.keyboard("{Home}");
  expect(screen.getByRole("menuitem", { name: "フォルダ管理..." })).toHaveFocus();
});

test("テーマのサブメニューでは、区切りを飛ばして全ラジオを移動できる", async () => {
  renderMenu();
  await openMenu();
  await userEvent.keyboard("{ArrowDown}{ArrowDown}{ArrowRight}");

  await userEvent.keyboard("{ArrowDown}{ArrowDown}{ArrowDown}{ArrowDown}");

  expect(screen.getByRole("menuitemradio", { name: "System" })).toHaveFocus();
});

test("← でサブメニューだけを閉じ、親の項目へフォーカスが戻る", async () => {
  renderMenu();
  await openMenu();
  await userEvent.keyboard("{ArrowDown}{ArrowRight}");

  await userEvent.keyboard("{ArrowLeft}");

  expect(querySubmenu("tag")).toBeNull();
  expect(queryTopMenu()).toBeInTheDocument();
  expect(group("tag")).toHaveFocus();
  expect(group("tag")).toHaveAttribute("aria-expanded", "false");
});

test("サブメニュー内の Esc はサブメニューだけを閉じ、親の項目へフォーカスが戻る", async () => {
  renderMenu();
  await openMenu();
  await userEvent.keyboard("{ArrowRight}");

  await userEvent.keyboard("{Escape}");

  expect(querySubmenu("file")).toBeNull();
  expect(queryTopMenu()).toBeInTheDocument();
  expect(group("file")).toHaveFocus();
});

test("1階層目の Esc でメニュー全体が閉じ、フォーカスがボタンへ戻る", async () => {
  renderMenu();
  await openMenu();

  await userEvent.keyboard("{Escape}");

  expect(queryTopMenu()).toBeNull();
  expect(button()).toHaveFocus();
  expect(button()).toHaveAttribute("aria-expanded", "false");
});

test("Esc は背後の document の keydown に伝えない", async () => {
  renderMenu();
  await openMenu();
  await userEvent.keyboard("{ArrowRight}");
  const behind = vi.fn();
  document.addEventListener("keydown", behind);

  await userEvent.keyboard("{Escape}{Escape}");
  document.removeEventListener("keydown", behind);

  expect(behind).not.toHaveBeenCalled();
});

test("サブメニューが開いたまま↑↓で別の項目へ移ると、サブメニューは閉じる", async () => {
  renderMenu();
  await openMenu();
  await userEvent.hover(group("file"));
  expect(submenu("file")).toBeInTheDocument();
  group("file").focus();

  await userEvent.keyboard("{ArrowDown}");

  expect(querySubmenu("file")).toBeNull();
  expect(group("tag")).toHaveFocus();
});

test("Tab は、1階層目でもサブメニューの中でも、メニュー全体を閉じてボタンへ戻す", async () => {
  renderMenu();
  await openMenu();
  await userEvent.keyboard("{Tab}");
  expect(queryTopMenu()).toBeNull();
  expect(button()).toHaveFocus();

  await openMenu();
  await userEvent.keyboard("{ArrowRight}{Tab}");
  expect(queryTopMenu()).toBeNull();
  expect(button()).toHaveFocus();
});

test("フォーカス中の葉を Enter で選べる", async () => {
  const { openDialog } = renderMenu();
  await openMenu();

  await userEvent.keyboard("{ArrowRight}{ArrowDown}{Enter}");

  expect(openDialog).toHaveBeenCalledWith("wbImport");
  expect(queryTopMenu()).toBeNull();
  expect(button()).toHaveFocus();
});

// ---- 閉じ方 ----

test("外側をクリックすると、サブメニューごと閉じ、フォーカスは奪わない", async () => {
  render(
    <>
      <input aria-label="外の入力欄" />
      <AppMenu
        appearance={DEFAULT_APPEARANCE}
        openDialog={vi.fn()}
        onSelectTheme={vi.fn()}
        onSelectMode={vi.fn()}
      />
    </>,
  );
  await openGroup("file");

  await userEvent.click(screen.getByLabelText("外の入力欄"));

  expect(queryTopMenu()).toBeNull();
  expect(querySubmenu("file")).toBeNull();
  expect(screen.getByLabelText("外の入力欄")).toHaveFocus();
});

test("サブメニューの余白をクリックしても閉じない", async () => {
  renderMenu();
  await openGroup("file");

  fireEvent.mouseDown(submenu("file"));

  expect(submenu("file")).toBeInTheDocument();
});

test("開いている間にボタンをもう一度押すと閉じる", async () => {
  renderMenu();
  await openGroup("file");

  await userEvent.click(button());

  expect(queryTopMenu()).toBeNull();
  expect(button()).toHaveAttribute("aria-expanded", "false");
});

test("閉じて開き直すと、サブメニューは閉じていて、最初の項目にフォーカスがある", async () => {
  renderMenu();
  await openGroup("help");
  await userEvent.keyboard("{Escape}{Escape}");

  await openMenu();

  expect(querySubmenu("help")).toBeNull();
  expect(group("file")).toHaveFocus();
});

test("ウィンドウの大きさが変わったら、メニュー全体を閉じる", async () => {
  renderMenu();
  await openGroup("file");

  fireEvent(window, new Event("resize"));

  expect(await screen.findByRole("button", { name: "メニュー" })).toBeInTheDocument();
  expect(queryTopMenu()).toBeNull();
  expect(querySubmenu("file")).toBeNull();
});

// ---- 位置 ----

type Rect = Partial<DOMRect> & { width?: number; height?: number };

function stubRects(rects: Record<string, Rect>) {
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (
    this: HTMLElement,
  ) {
    const id = this.dataset.testid ?? "";
    const r = rects[id] ?? {};
    return {
      left: 0,
      top: 0,
      right: (r.left ?? 0) + (r.width ?? 0),
      bottom: (r.top ?? 0) + (r.height ?? 0),
      width: 0,
      height: 0,
      ...r,
    } as DOMRect;
  });
}

test("ボタンの直下に、右端をそろえて開く", async () => {
  vi.stubGlobal("innerWidth", 1000);
  vi.stubGlobal("innerHeight", 600);
  stubRects({
    "app-menu-button": { left: 940, right: 980, top: 10, bottom: 40 },
    "app-menu": { width: 200, height: 150 },
  });
  renderMenu();
  await openMenu();

  expect(topMenu()).toHaveStyle({ left: "780px", top: "44px", visibility: "visible" });
});

test("左にはみ出す場合は、画面の内側へ補正する", async () => {
  vi.stubGlobal("innerWidth", 1000);
  vi.stubGlobal("innerHeight", 600);
  stubRects({
    "app-menu-button": { left: 50, right: 90, top: 10, bottom: 40 },
    "app-menu": { width: 200, height: 150 },
  });
  renderMenu();
  await openMenu();

  expect(topMenu()).toHaveStyle({ left: "4px" });
});

test("サブメニューは、1階層目の左側に、その項目の高さをそろえて開く", async () => {
  vi.stubGlobal("innerWidth", 1000);
  vi.stubGlobal("innerHeight", 600);
  stubRects({
    "app-menu-button": { left: 940, right: 980, top: 10, bottom: 40 },
    "app-menu": { left: 780, right: 980, top: 44, width: 200, height: 150 },
    "app-menu-group-theme": { top: 100, height: 28 },
    "app-submenu-theme": { width: 160, height: 200 },
  });
  renderMenu();
  await openMenu();
  await userEvent.click(group("theme"));

  expect(submenu("theme")).toHaveStyle({ left: "620px", top: "96px", visibility: "visible" });
});

test("左に収まらないときは、サブメニューを1階層目の右側へ開く", async () => {
  vi.stubGlobal("innerWidth", 1000);
  vi.stubGlobal("innerHeight", 600);
  stubRects({
    "app-menu-button": { left: 100, right: 140, top: 10, bottom: 40 },
    "app-menu": { left: 10, right: 150, top: 44, width: 140, height: 150 },
    "app-menu-group-file": { top: 50, height: 28 },
    "app-submenu-file": { width: 200, height: 120 },
  });
  renderMenu();
  await openMenu();
  await userEvent.click(group("file"));

  expect(submenu("file")).toHaveStyle({ left: "150px", top: "46px" });
});

test("サブメニューが下にはみ出す場合は、上へ補正する", async () => {
  vi.stubGlobal("innerWidth", 1000);
  vi.stubGlobal("innerHeight", 300);
  stubRects({
    "app-menu-button": { left: 940, right: 980, top: 10, bottom: 40 },
    "app-menu": { left: 780, right: 980, top: 44, width: 200, height: 150 },
    "app-menu-group-help": { top: 240, height: 28 },
    "app-submenu-help": { width: 160, height: 120 },
  });
  renderMenu();
  await openMenu();
  await userEvent.click(group("help"));

  expect(submenu("help")).toHaveStyle({ top: "176px" });
});

// ---- ダイアログが開けない状況 ----

function Harness() {
  const { active, open } = useActiveDialog();
  return (
    <>
      <AppMenu
        appearance={DEFAULT_APPEARANCE}
        openDialog={open}
        onSelectTheme={vi.fn()}
        onSelectMode={vi.fn()}
      />
      <span data-testid="active">{active ?? "none"}</span>
    </>
  );
}

test("閉じられないダイアログが開いている間は、項目を選んでも開かず、メニューは閉じる", async () => {
  const release = registerUndismissibleModal();
  render(<Harness />);
  await openGroup("settings");

  await userEvent.click(screen.getByRole("menuitem", { name: "設定..." }));

  expect(screen.getByTestId("active")).toHaveTextContent("none");
  expect(queryTopMenu()).toBeNull();
  release();
});

test("閉じられるダイアログなら、実際の useActiveDialog 経由で開く", async () => {
  render(<Harness />);
  await openGroup("help");

  await userEvent.click(screen.getByRole("menuitem", { name: "動作ログ..." }));

  expect(screen.getByTestId("active")).toHaveTextContent("log");
});
