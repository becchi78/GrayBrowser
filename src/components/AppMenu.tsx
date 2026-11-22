import { useCallback, useEffect, useId, useLayoutEffect, useRef, useState } from "react";
import type { RefObject } from "react";
import { createPortal } from "react-dom";
import { openDevtools, quitApp, runManualBackup } from "../api";
import {
  MODES,
  MODE_LABELS,
  THEMES,
  THEME_LABELS,
  type Appearance,
  type Mode,
  type Theme,
} from "../lib/appearance";
import { placeDropdownRightAligned, placeSubmenu, type MenuPosition } from "../lib/contextMenu";
import { focusFirstItem, moveMenuFocus, type FocusMove } from "../lib/menuFocus";
import type { DialogKind } from "../useActiveDialog";
import { useDeveloperMode } from "../useDeveloperMode";

interface Props {
  openDialog: (kind: DialogKind) => void;
  appearance: Appearance;
  onSelectTheme: (theme: Theme) => void;
  onSelectMode: (mode: Mode) => void;
}

interface MenuLeaf {
  id: string;
  label: string;
  /** 指定すると menuitemradio になり、その値が選択状態を示す。 */
  checked?: boolean;
  onSelect: () => void;
}

type MenuEntry = MenuLeaf | "separator";

interface MenuGroup {
  /** data-testid(`app-menu-group-<id>`)に使う。 */
  id: string;
  label: string;
  entries: MenuEntry[];
}

// 画面端との余白。ContextMenu の補正と同じ値。
const VIEWPORT_MARGIN = 4;
// ボタンとドロップダウンの間の隙間。
const ANCHOR_GAP = 4;
// `.app-menu` の padding(var(--space-1))。サブメニューの最初の項目を、親の項目の
// 高さにそろえるために引く。
const SUBMENU_TOP_OFFSET = 4;

const KEY_MOVES: Record<string, FocusMove> = {
  ArrowDown: "next",
  ArrowUp: "prev",
  Home: "first",
  End: "last",
};

function reportFailure(e: unknown) {
  console.error(e);
}

function MenuIcon() {
  return (
    <svg
      className="app-menu-icon"
      viewBox="0 0 24 24"
      fill="currentColor"
      aria-hidden="true"
      focusable="false"
    >
      <rect x="3" y="5.25" width="18" height="2" rx="1" />
      <rect x="3" y="11" width="18" height="2" rx="1" />
      <rect x="3" y="16.75" width="18" height="2" rx="1" />
    </svg>
  );
}

function ChevronIcon() {
  return (
    <svg
      className="app-menu-chevron"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="3"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
    >
      <path d="M9 5l7 7-7 7" />
    </svg>
  );
}

function CheckIcon() {
  return (
    <svg
      className="app-menu-check"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="3"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
    >
      <path d="M5 12.5l4.5 4.5L19 7.5" />
    </svg>
  );
}

// 検索行の右端に置く、アプリ全体のメニュー(ハンバーガー)。1階層目はグループの一覧で、
// 項目を開くとその横にサブメニューが出る(以前のネイティブメニューバーと同じ構成)。
// ドロップダウンは ContextMenu と同じく document.body へ portal で描画し、一覧などの
// position / overflow の効く祖先の影響を受けないようにする。
export function AppMenu({ openDialog, appearance, onSelectTheme, onSelectMode }: Props) {
  const [open, setOpen] = useState(false);
  const developerMode = useDeveloperMode();
  const buttonRef = useRef<HTMLButtonElement>(null);

  const closeAndRestoreFocus = useCallback(() => {
    setOpen(false);
    buttonRef.current?.focus();
  }, []);
  // 外側のクリックでは、利用者が操作した先を優先し、フォーカスはボタンへ戻さない。
  const closeSilently = useCallback(() => setOpen(false), []);

  const dialog = (id: string, label: string, kind: DialogKind): MenuLeaf => ({
    id,
    label,
    onSelect: () => openDialog(kind),
  });

  const groups: MenuGroup[] = [
    {
      id: "file",
      label: "ファイル",
      entries: [
        dialog("folder", "フォルダ管理...", "folder"),
        dialog("wbImport", ".wbインポート...", "wbImport"),
        {
          id: "backup",
          label: "データベースをバックアップ",
          onSelect: () => runManualBackup().catch(reportFailure),
        },
        { id: "quit", label: "終了", onSelect: () => quitApp().catch(reportFailure) },
      ],
    },
    { id: "tag", label: "タグ", entries: [dialog("tagBar", "タグの編集...", "tagBar")] },
    {
      id: "theme",
      label: "テーマ",
      entries: [
        ...THEMES.map((t) => ({
          id: `theme-${t}`,
          label: THEME_LABELS[t],
          checked: appearance.theme === t,
          onSelect: () => onSelectTheme(t),
        })),
        "separator" as const,
        ...MODES.map((m) => ({
          id: `mode-${m}`,
          label: MODE_LABELS[m],
          checked: appearance.mode === m,
          onSelect: () => onSelectMode(m),
        })),
      ],
    },
    { id: "settings", label: "設定", entries: [dialog("settings", "設定...", "settings")] },
    {
      id: "help",
      label: "ヘルプ",
      entries: [
        dialog("log", "動作ログ...", "log"),
        // 開発者向けの項目は、開発者モードがオンのときだけ出す。
        ...(developerMode
          ? [
              dialog("diagnostics", "診断情報...", "diagnostics"),
              dialog("dbTools", "データベースツール...", "dbTools"),
              dialog("scanReport", "スキャン診断...", "scanReport"),
            ]
          : []),
        dialog("update", "更新の確認...", "update"),
        ...(developerMode
          ? [dialog("versionPin", "バージョンを指定してインストール...", "versionPin")]
          : []),
        dialog("about", "バージョン情報", "about"),
        ...(developerMode
          ? [
              "separator" as const,
              {
                id: "devtools",
                label: "開発者ツールを開く",
                onSelect: () => openDevtools().catch(reportFailure),
              },
            ]
          : []),
      ],
    },
  ];

  return (
    <>
      <button
        ref={buttonRef}
        type="button"
        className="app-menu-button"
        data-testid="app-menu-button"
        aria-label="メニュー"
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
      >
        <MenuIcon />
      </button>
      {open && (
        <AppMenuDropdown
          anchor={buttonRef}
          groups={groups}
          onClose={closeAndRestoreFocus}
          onDismissOutside={closeSilently}
        />
      )}
    </>
  );
}

interface DropdownProps {
  anchor: RefObject<HTMLButtonElement | null>;
  groups: MenuGroup[];
  onClose: () => void;
  onDismissOutside: () => void;
}

interface OpenSubmenu {
  group: string;
  /** キーボードで開いたときだけ、サブメニューの最初の項目へフォーカスを移す。 */
  focusFirst: boolean;
}

function AppMenuDropdown({ anchor, groups, onClose, onDismissOutside }: DropdownProps) {
  const menuRef = useRef<HTMLDivElement>(null);
  const [placed, setPlaced] = useState<MenuPosition | null>(null);
  const [submenu, setSubmenu] = useState<OpenSubmenu | null>(null);

  // サイズは描画後でないと分からないため、測ってから補正する。補正前の位置が
  // 一瞬見えないよう、済むまで visibility を隠す。位置はボタンの直下・右揃え
  // (ボタンが検索行の右端にあるため)。
  useLayoutEffect(() => {
    const menu = menuRef.current;
    const button = anchor.current;
    if (!menu || !button) return;
    const size = menu.getBoundingClientRect();
    setPlaced(
      placeDropdownRightAligned(
        button.getBoundingClientRect(),
        { width: size.width, height: size.height },
        { width: window.innerWidth, height: window.innerHeight },
        ANCHOR_GAP,
        VIEWPORT_MARGIN,
      ),
    );
  }, [anchor]);

  // visibility: hidden の要素にはフォーカスできないので、位置が確定してから。
  const isPlaced = placed !== null;
  useEffect(() => {
    if (isPlaced) focusFirstItem(menuRef.current);
  }, [isPlaced]);

  useEffect(() => {
    function onMouseDown(e: MouseEvent) {
      if (!(e.target instanceof Node)) return;
      // サブメニューは1階層目の DOM の内側にあるので、これで両方を含む。
      if (menuRef.current?.contains(e.target)) return;
      // ボタン自身の押下は、ボタンの onClick が開閉を切り替える。ここで閉じると
      // その click で開き直してしまう。
      if (anchor.current?.contains(e.target)) return;
      onDismissOutside();
    }
    function onKeyDown(e: KeyboardEvent) {
      const top = menuRef.current;
      if (!top) return;
      const active = document.activeElement instanceof HTMLElement ? document.activeElement : null;
      const current = active?.closest<HTMLElement>('[role="menu"]') ?? top;
      const inSubmenu = current !== top;

      if (e.key === "Tab" || (e.key === "Escape" && !inSubmenu)) {
        e.preventDefault();
        // 背後の document の keydown リスナー(ダイアログの Escape など)へ伝えない。
        e.stopPropagation();
        onClose();
        return;
      }
      if ((e.key === "Escape" || e.key === "ArrowLeft") && inSubmenu) {
        e.preventDefault();
        e.stopPropagation();
        // サブメニューだけを閉じ、親の項目(同じ wrapper の先頭)へフォーカスを戻す。
        const parentItem = current.parentElement?.firstElementChild;
        setSubmenu(null);
        if (parentItem instanceof HTMLElement) parentItem.focus();
        return;
      }
      const group = !inSubmenu && active ? active.dataset.group : undefined;
      if (group && (e.key === "ArrowRight" || e.key === "Enter" || e.key === " ")) {
        // click を起こさせず、ここで開く(マウスの click と区別するため)。
        e.preventDefault();
        setSubmenu({ group, focusFirst: true });
        return;
      }
      const move = KEY_MOVES[e.key];
      if (move) {
        e.preventDefault();
        // 1階層目で別の項目へ移るとき、開いていたサブメニューは閉じる。
        if (!inSubmenu) setSubmenu(null);
        moveMenuFocus(current, move);
      }
    }
    // 項目が多いと画面に収まらずメニュー自体がスクロールするため、ContextMenu と
    // 違いスクロールやホイールでは閉じない。
    document.addEventListener("mousedown", onMouseDown, true);
    document.addEventListener("keydown", onKeyDown, true);
    window.addEventListener("blur", onDismissOutside);
    window.addEventListener("resize", onDismissOutside);
    return () => {
      document.removeEventListener("mousedown", onMouseDown, true);
      document.removeEventListener("keydown", onKeyDown, true);
      window.removeEventListener("blur", onDismissOutside);
      window.removeEventListener("resize", onDismissOutside);
    };
  }, [anchor, onClose, onDismissOutside]);

  const shown = placed ?? { x: 0, y: 0 };
  return createPortal(
    <div
      ref={menuRef}
      className="app-menu"
      role="menu"
      aria-label="メニュー"
      data-testid="app-menu"
      style={{
        left: shown.x,
        top: shown.y,
        visibility: placed ? "visible" : "hidden",
        maxHeight: `calc(100vh - ${VIEWPORT_MARGIN * 2}px)`,
      }}
      onContextMenu={(e) => e.preventDefault()}
    >
      {groups.map((group) => (
        <GroupItem
          key={group.id}
          group={group}
          topMenuRef={menuRef}
          open={submenu?.group === group.id}
          focusFirst={submenu?.group === group.id && submenu.focusFirst}
          // ホバーで開き、別のグループへ移れば切り替わる。メニューの外へ出ても閉じない
          // (斜めに動かしたときに消えないようにするため)。
          onHover={() => setSubmenu({ group: group.id, focusFirst: false })}
          onClose={onClose}
        />
      ))}
    </div>,
    document.body,
  );
}

interface GroupItemProps {
  group: MenuGroup;
  topMenuRef: RefObject<HTMLDivElement | null>;
  open: boolean;
  focusFirst: boolean;
  onHover: () => void;
  onClose: () => void;
}

// 1階層目の項目。サブメニューは role=none の wrapper の中に、項目と並べて置く
// (WAI-ARIA のメニューの例と同じ形)ため、アクセシビリティツリー上で親子が保たれる。
function GroupItem({ group, topMenuRef, open, focusFirst, onHover, onClose }: GroupItemProps) {
  const submenuId = useId();
  const itemRef = useRef<HTMLButtonElement>(null);
  return (
    <div role="none">
      <button
        ref={itemRef}
        type="button"
        role="menuitem"
        className="app-menu-item"
        data-testid={`app-menu-group-${group.id}`}
        data-group={group.id}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-controls={open ? submenuId : undefined}
        // ホバーした項目へフォーカスを追従させ、強調を常に1か所にする。
        onMouseEnter={(e) => {
          e.currentTarget.focus();
          onHover();
        }}
        onClick={onHover}
      >
        <span className="app-menu-group-label">{group.label}</span>
        <ChevronIcon />
      </button>
      {open && (
        <Submenu
          id={submenuId}
          group={group}
          itemRef={itemRef}
          topMenuRef={topMenuRef}
          focusFirst={focusFirst}
          onClose={onClose}
        />
      )}
    </div>
  );
}

interface SubmenuProps {
  id: string;
  group: MenuGroup;
  itemRef: RefObject<HTMLButtonElement | null>;
  topMenuRef: RefObject<HTMLDivElement | null>;
  focusFirst: boolean;
  onClose: () => void;
}

function Submenu({ id, group, itemRef, topMenuRef, focusFirst, onClose }: SubmenuProps) {
  const menuRef = useRef<HTMLDivElement>(null);
  const [placed, setPlaced] = useState<MenuPosition | null>(null);

  // 1階層目の項目の横に出す。ボタンが右端にあるので左側を優先し、収まらなければ
  // 右側へ。大きさは描画後でないと分からないため、測ってから確定する。
  useLayoutEffect(() => {
    const menu = menuRef.current;
    const item = itemRef.current;
    const parent = topMenuRef.current;
    if (!menu || !item || !parent) return;
    const size = menu.getBoundingClientRect();
    const { x, y } = placeSubmenu(
      parent.getBoundingClientRect(),
      item.getBoundingClientRect().top - SUBMENU_TOP_OFFSET,
      { width: size.width, height: size.height },
      { width: window.innerWidth, height: window.innerHeight },
      VIEWPORT_MARGIN,
    );
    setPlaced({ x, y });
  }, [itemRef, topMenuRef]);

  const isPlaced = placed !== null;
  useEffect(() => {
    if (isPlaced && focusFirst) focusFirstItem(menuRef.current);
  }, [isPlaced, focusFirst]);

  const shown = placed ?? { x: 0, y: 0 };
  return (
    <div
      ref={menuRef}
      id={id}
      className="app-menu app-submenu"
      role="menu"
      aria-label={group.label}
      data-testid={`app-submenu-${group.id}`}
      style={{
        left: shown.x,
        top: shown.y,
        visibility: placed ? "visible" : "hidden",
        maxHeight: `calc(100vh - ${VIEWPORT_MARGIN * 2}px)`,
      }}
    >
      {group.entries.map((entry, i) => {
        if (entry === "separator") {
          return <div key={`sep-${i}`} role="separator" className="app-menu-separator" />;
        }
        const isRadio = entry.checked !== undefined;
        return (
          <button
            key={entry.id}
            type="button"
            role={isRadio ? "menuitemradio" : "menuitem"}
            aria-checked={isRadio ? entry.checked : undefined}
            className="app-menu-item"
            data-testid={`app-menu-item-${entry.id}`}
            onMouseEnter={(e) => e.currentTarget.focus()}
            onClick={() => {
              // 先にフォーカスをボタンへ戻す。項目の DOM は閉じると消えるため、
              // 戻さないとフォーカスが失われる。
              onClose();
              entry.onSelect();
            }}
          >
            <span className="app-menu-check-slot">{entry.checked ? <CheckIcon /> : null}</span>
            {entry.label}
          </button>
        );
      })}
    </div>
  );
}
