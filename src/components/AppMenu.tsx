import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import type { RefObject } from "react";
import { createPortal } from "react-dom";
import { quitApp, runManualBackup } from "../api";
import {
  MODES,
  MODE_LABELS,
  THEMES,
  THEME_LABELS,
  type Appearance,
  type Mode,
  type Theme,
} from "../lib/appearance";
import { clampMenuPosition, type MenuPosition } from "../lib/contextMenu";
import { focusFirstItem, moveMenuFocus, type FocusMove } from "../lib/menuFocus";
import type { DialogKind } from "../useActiveDialog";

interface Props {
  openDialog: (kind: DialogKind) => void;
  appearance: Appearance;
  onSelectTheme: (theme: Theme) => void;
  onSelectMode: (mode: Mode) => void;
}

interface MenuEntry {
  id: string;
  label: string;
  /** 指定すると menuitemradio になり、その値が選択状態を示す。 */
  checked?: boolean;
  onSelect: () => void;
}

interface MenuGroup {
  label: string;
  entries: MenuEntry[];
}

// 画面端との余白。ContextMenu の補正と同じ値。
const VIEWPORT_MARGIN = 4;
// ボタンとドロップダウンの間の隙間。
const ANCHOR_GAP = 4;

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

// 検索行の左端に置く、アプリ全体のメニュー(ハンバーガー)。
// ドロップダウンは ContextMenu と同じく document.body へ portal で描画し、一覧などの
// position / overflow の効く祖先の影響を受けないようにする。
export function AppMenu({ openDialog, appearance, onSelectTheme, onSelectMode }: Props) {
  const [open, setOpen] = useState(false);
  const buttonRef = useRef<HTMLButtonElement>(null);

  const closeAndRestoreFocus = useCallback(() => {
    setOpen(false);
    buttonRef.current?.focus();
  }, []);
  // 外側のクリックでは、利用者が操作した先を優先し、フォーカスはボタンへ戻さない。
  const closeSilently = useCallback(() => setOpen(false), []);

  const dialog = (id: string, label: string, kind: DialogKind): MenuEntry => ({
    id,
    label,
    onSelect: () => openDialog(kind),
  });

  const groups: MenuGroup[] = [
    {
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
    { label: "タグ", entries: [dialog("tagBar", "タグの編集...", "tagBar")] },
    {
      label: "スタイル",
      entries: [
        ...THEMES.map((t) => ({
          id: `theme-${t}`,
          label: THEME_LABELS[t],
          checked: appearance.theme === t,
          onSelect: () => onSelectTheme(t),
        })),
        ...MODES.map((m) => ({
          id: `mode-${m}`,
          label: MODE_LABELS[m],
          checked: appearance.mode === m,
          onSelect: () => onSelectMode(m),
        })),
      ],
    },
    { label: "設定", entries: [dialog("settings", "設定...", "settings")] },
    {
      label: "ヘルプ",
      entries: [
        dialog("log", "動作ログ...", "log"),
        dialog("update", "更新の確認...", "update"),
        dialog("about", "バージョン情報", "about"),
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

function AppMenuDropdown({ anchor, groups, onClose, onDismissOutside }: DropdownProps) {
  const menuRef = useRef<HTMLDivElement>(null);
  const [placed, setPlaced] = useState<MenuPosition | null>(null);

  // サイズは描画後でないと分からないため、測ってから補正する。補正前の位置が
  // 一瞬見えないよう、済むまで visibility を隠す。位置はボタンの直下・左揃え。
  useLayoutEffect(() => {
    const menu = menuRef.current;
    const button = anchor.current;
    if (!menu || !button) return;
    const rect = button.getBoundingClientRect();
    const size = menu.getBoundingClientRect();
    setPlaced(
      clampMenuPosition(
        { x: rect.left, y: rect.bottom + ANCHOR_GAP },
        { width: size.width, height: size.height },
        { width: window.innerWidth, height: window.innerHeight },
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
      if (menuRef.current?.contains(e.target)) return;
      // ボタン自身の押下は、ボタンの onClick が開閉を切り替える。ここで閉じると
      // その click で開き直してしまう。
      if (anchor.current?.contains(e.target)) return;
      onDismissOutside();
    }
    function onKeyDown(e: KeyboardEvent) {
      if (e.key === "Escape" || e.key === "Tab") {
        e.preventDefault();
        // 背後の document の keydown リスナー(ダイアログの Escape など)へ伝えない。
        e.stopPropagation();
        onClose();
        return;
      }
      const move = KEY_MOVES[e.key];
      if (move) {
        e.preventDefault();
        moveMenuFocus(menuRef.current, move);
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
      {groups.map((group, i) => (
        <div key={group.label}>
          {i > 0 && <div role="separator" className="app-menu-separator" />}
          <div role="group" aria-label={group.label}>
            {/* グループ名は group の aria-label で伝わるため、見出しは読み上げない。 */}
            <div className="app-menu-heading" aria-hidden="true">
              {group.label}
            </div>
            {group.entries.map((entry) => {
              const isRadio = entry.checked !== undefined;
              return (
                <button
                  key={entry.id}
                  type="button"
                  role={isRadio ? "menuitemradio" : "menuitem"}
                  aria-checked={isRadio ? entry.checked : undefined}
                  className="app-menu-item"
                  data-testid={`app-menu-item-${entry.id}`}
                  // ホバーした項目へフォーカスを追従させ、強調を常に1か所にする。
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
        </div>
      ))}
    </div>,
    document.body,
  );
}
