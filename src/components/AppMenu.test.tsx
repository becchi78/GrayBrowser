// AppMenu: ハンバーガーのドロップダウンの内容・各項目の動作・キーボード操作・位置。
import { fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import * as api from "../api";
import type { Appearance, Mode, Theme } from "../lib/appearance";
import { registerUndismissibleModal } from "../undismissibleModals";
import { useActiveDialog, type DialogKind } from "../useActiveDialog";
import { AppMenu } from "./AppMenu";

vi.mock("../api", () => ({
  runManualBackup: vi.fn(),
  quitApp: vi.fn(),
}));

const DEFAULT_APPEARANCE: Appearance = { theme: "console", mode: "system" };

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
const menu = () => screen.getByRole("menu");
const queryMenu = () => screen.queryByRole("menu");

async function openMenu() {
  await userEvent.click(button());
}

const ENTRY_LABELS = [
  "フォルダ管理...",
  ".wbインポート...",
  "データベースをバックアップ",
  "終了",
  "タグの編集...",
  "Navy Console",
  "Fluent Mica",
  "Dark Cinema",
  "System",
  "Light",
  "Dark",
  "設定...",
  "動作ログ...",
  "更新の確認...",
  "バージョン情報",
];

beforeEach(() => {
  vi.spyOn(console, "error").mockImplementation(() => {});
  vi.mocked(api.runManualBackup).mockResolvedValue(undefined);
  vi.mocked(api.quitApp).mockResolvedValue(undefined);
});

afterEach(() => {
  vi.restoreAllMocks();
});

test("閉じている間は、ボタンだけがあり、メニューは無い", () => {
  renderMenu();
  expect(button()).toHaveAttribute("aria-haspopup", "menu");
  expect(button()).toHaveAttribute("aria-expanded", "false");
  expect(queryMenu()).toBeNull();
});

test("ボタンを押すと開き、aria-expanded が true になる", async () => {
  renderMenu();
  await openMenu();
  expect(menu()).toBeInTheDocument();
  expect(button()).toHaveAttribute("aria-expanded", "true");
});

test("全項目が、現行のラベルのまま、この順で並ぶ", async () => {
  renderMenu();
  await openMenu();
  const labels = Array.from(menu().querySelectorAll('[role^="menuitem"]')).map(
    (el) => el.textContent,
  );
  expect(labels).toEqual(ENTRY_LABELS);
});

test("グループ名が group として付き、グループの間に区切りがある", async () => {
  renderMenu();
  await openMenu();
  const groups = within(menu()).getAllByRole("group");
  expect(groups.map((g) => g.getAttribute("aria-label"))).toEqual([
    "ファイル",
    "タグ",
    "スタイル",
    "設定",
    "ヘルプ",
  ]);
  expect(within(menu()).getAllByRole("separator")).toHaveLength(4);
});

test("Enter でも Space でもボタンから開ける", async () => {
  renderMenu();
  button().focus();
  await userEvent.keyboard("{Enter}");
  expect(menu()).toBeInTheDocument();

  await userEvent.keyboard("{Escape}");
  expect(queryMenu()).toBeNull();

  await userEvent.keyboard(" ");
  expect(menu()).toBeInTheDocument();
});

test("開くと最初の項目にフォーカスがある", async () => {
  renderMenu();
  await openMenu();
  expect(screen.getByRole("menuitem", { name: "フォルダ管理..." })).toHaveFocus();
});

test.each<[string, DialogKind]>([
  ["フォルダ管理...", "folder"],
  [".wbインポート...", "wbImport"],
  ["タグの編集...", "tagBar"],
  ["設定...", "settings"],
  ["動作ログ...", "log"],
  ["更新の確認...", "update"],
  ["バージョン情報", "about"],
])("「%s」は %s のダイアログを開いてメニューを閉じる", async (label, kind) => {
  const { openDialog } = renderMenu();
  await openMenu();

  await userEvent.click(screen.getByRole("menuitem", { name: label }));

  expect(openDialog).toHaveBeenCalledTimes(1);
  expect(openDialog).toHaveBeenCalledWith(kind);
  expect(queryMenu()).toBeNull();
});

test("「データベースをバックアップ」はバックアップのコマンドを呼ぶ", async () => {
  const { openDialog } = renderMenu();
  await openMenu();

  await userEvent.click(screen.getByRole("menuitem", { name: "データベースをバックアップ" }));

  expect(api.runManualBackup).toHaveBeenCalledTimes(1);
  expect(api.quitApp).not.toHaveBeenCalled();
  expect(openDialog).not.toHaveBeenCalled();
  expect(queryMenu()).toBeNull();
});

test("「終了」は終了のコマンドを呼ぶ", async () => {
  renderMenu();
  await openMenu();

  await userEvent.click(screen.getByRole("menuitem", { name: "終了" }));

  expect(api.quitApp).toHaveBeenCalledTimes(1);
  expect(api.runManualBackup).not.toHaveBeenCalled();
});

test("コマンドが失敗しても例外にせず、記録に残す", async () => {
  vi.mocked(api.runManualBackup).mockRejectedValue(new Error("x"));
  vi.mocked(api.quitApp).mockRejectedValue(new Error("y"));
  renderMenu();

  await openMenu();
  await userEvent.click(screen.getByRole("menuitem", { name: "データベースをバックアップ" }));
  await openMenu();
  await userEvent.click(screen.getByRole("menuitem", { name: "終了" }));

  expect(console.error).toHaveBeenCalledTimes(2);
});

test("現在のテーマと外観だけが aria-checked になる", async () => {
  renderMenu({ theme: "cinema", mode: "light" });
  await openMenu();

  const checked = screen
    .getAllByRole("menuitemradio")
    .filter((el) => el.getAttribute("aria-checked") === "true")
    .map((el) => el.textContent);
  expect(checked).toEqual(["Dark Cinema", "Light"]);
  expect(screen.getAllByRole("menuitemradio")).toHaveLength(6);
});

test("テーマの項目を選ぶと選択関数を呼び、外観は呼ばずにメニューを閉じる", async () => {
  const { onSelectTheme, onSelectMode } = renderMenu();
  await openMenu();

  await userEvent.click(screen.getByRole("menuitemradio", { name: "Fluent Mica" }));

  expect(onSelectTheme).toHaveBeenCalledWith("fluent");
  expect(onSelectMode).not.toHaveBeenCalled();
  expect(queryMenu()).toBeNull();
});

test("外観の項目を選ぶと選択関数を呼び、テーマは呼ばない", async () => {
  const { onSelectTheme, onSelectMode } = renderMenu();
  await openMenu();

  await userEvent.click(screen.getByRole("menuitemradio", { name: "Dark" }));

  expect(onSelectMode).toHaveBeenCalledWith("dark");
  expect(onSelectTheme).not.toHaveBeenCalled();
});

test("↓↑で項目を移動し、端で反対側へ循環する", async () => {
  renderMenu();
  await openMenu();
  const first = screen.getByRole("menuitem", { name: "フォルダ管理..." });
  const last = screen.getByRole("menuitem", { name: "バージョン情報" });

  await userEvent.keyboard("{ArrowDown}");
  expect(screen.getByRole("menuitem", { name: ".wbインポート..." })).toHaveFocus();

  await userEvent.keyboard("{ArrowUp}{ArrowUp}");
  expect(last).toHaveFocus();

  await userEvent.keyboard("{ArrowDown}");
  expect(first).toHaveFocus();
});

test("ラジオ項目にも矢印キーで移動できる", async () => {
  renderMenu();
  await openMenu();
  screen.getByRole("menuitem", { name: "タグの編集..." }).focus();

  await userEvent.keyboard("{ArrowDown}");

  expect(screen.getByRole("menuitemradio", { name: "Navy Console" })).toHaveFocus();
});

test("Home で先頭、End で末尾へ移る", async () => {
  renderMenu();
  await openMenu();

  await userEvent.keyboard("{End}");
  expect(screen.getByRole("menuitem", { name: "バージョン情報" })).toHaveFocus();

  await userEvent.keyboard("{Home}");
  expect(screen.getByRole("menuitem", { name: "フォルダ管理..." })).toHaveFocus();
});

test("フォーカス中の項目を Enter で選べる", async () => {
  const { openDialog } = renderMenu();
  await openMenu();

  await userEvent.keyboard("{ArrowDown}{Enter}");

  expect(openDialog).toHaveBeenCalledWith("wbImport");
});

test("Esc で閉じ、フォーカスがボタンへ戻る", async () => {
  renderMenu();
  await openMenu();

  await userEvent.keyboard("{Escape}");

  expect(queryMenu()).toBeNull();
  expect(button()).toHaveFocus();
  expect(button()).toHaveAttribute("aria-expanded", "false");
});

test("Esc は背後の document の keydown に伝えない", async () => {
  renderMenu();
  await openMenu();
  const behind = vi.fn();
  document.addEventListener("keydown", behind);

  await userEvent.keyboard("{Escape}");
  document.removeEventListener("keydown", behind);

  expect(behind).not.toHaveBeenCalled();
});

test("Tab でメニューが閉じ、フォーカスはボタンへ戻る", async () => {
  renderMenu();
  await openMenu();

  await userEvent.keyboard("{Tab}");

  expect(queryMenu()).toBeNull();
  expect(button()).toHaveFocus();
});

test("項目を選んだ後は、フォーカスがボタンへ戻る", async () => {
  renderMenu();
  await openMenu();

  await userEvent.click(screen.getByRole("menuitemradio", { name: "Light" }));

  expect(button()).toHaveFocus();
});

test("外側をクリックすると閉じ、フォーカスは奪わない", async () => {
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
  await openMenu();

  await userEvent.click(screen.getByLabelText("外の入力欄"));

  expect(queryMenu()).toBeNull();
  expect(screen.getByLabelText("外の入力欄")).toHaveFocus();
});

test("メニュー内の余白(項目以外)をクリックしても閉じない", async () => {
  renderMenu();
  await openMenu();

  fireEvent.mouseDown(menu());

  expect(menu()).toBeInTheDocument();
});

test("開いている間にボタンをもう一度押すと閉じる", async () => {
  renderMenu();
  await openMenu();

  await userEvent.click(button());

  expect(queryMenu()).toBeNull();
  expect(button()).toHaveAttribute("aria-expanded", "false");
});

test("閉じて開き直すと、また最初の項目にフォーカスがある", async () => {
  renderMenu();
  await openMenu();
  await userEvent.keyboard("{End}{Escape}");

  await openMenu();

  expect(screen.getByRole("menuitem", { name: "フォルダ管理..." })).toHaveFocus();
});

test("ウィンドウの大きさが変わったら閉じる", async () => {
  renderMenu();
  await openMenu();

  fireEvent(window, new Event("resize"));

  expect(await screen.findByRole("button", { name: "メニュー" })).toBeInTheDocument();
  expect(queryMenu()).toBeNull();
});

// ---- 位置 ----

function stubRects(buttonRect: Partial<DOMRect>, menuSize: { width: number; height: number }) {
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (
    this: HTMLElement,
  ) {
    if (this.dataset.testid === "app-menu-button") return { ...buttonRect } as DOMRect;
    if (this.dataset.testid === "app-menu") return { ...menuSize } as DOMRect;
    return {} as DOMRect;
  });
}

test("ボタンの直下に、左端をそろえて開く", async () => {
  stubRects({ left: 12, bottom: 40 }, { width: 200, height: 300 });
  renderMenu();
  await openMenu();

  expect(menu()).toHaveStyle({ left: "12px", top: "44px", visibility: "visible" });
});

test("右下にはみ出す場合は、画面の内側へ補正する", async () => {
  vi.stubGlobal("innerWidth", 1000);
  vi.stubGlobal("innerHeight", 600);
  stubRects({ left: 900, bottom: 500 }, { width: 200, height: 300 });
  renderMenu();
  await openMenu();

  expect(menu()).toHaveStyle({ left: "796px", top: "296px" });
  vi.unstubAllGlobals();
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
  await openMenu();

  await userEvent.click(screen.getByRole("menuitem", { name: "設定..." }));

  expect(screen.getByTestId("active")).toHaveTextContent("none");
  expect(queryMenu()).toBeNull();
  release();
});

test("閉じられるダイアログなら、実際の useActiveDialog 経由で開く", async () => {
  render(<Harness />);
  await openMenu();

  await userEvent.click(screen.getByRole("menuitem", { name: "動作ログ..." }));

  expect(screen.getByTestId("active")).toHaveTextContent("log");
});
