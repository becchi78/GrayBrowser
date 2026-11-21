import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { clampMenuPosition, type MenuPosition } from "../lib/contextMenu";
import { focusFirstItem, moveMenuFocus } from "../lib/menuFocus";

export interface ContextMenuItem {
  label: string;
  danger?: boolean;
  /** true なら押せず、矢印キーの移動対象からも外れる。 */
  disabled?: boolean;
  onSelect: () => void;
}

interface Props {
  /** ポインタの位置(viewport 座標)。画面外にはみ出す場合は内側へ補正する。 */
  position: MenuPosition;
  items: ContextMenuItem[];
  onClose: () => void;
}

// 一覧のスクロール領域や重複パネルなど position / overflow の効く祖先の
// 影響を受けないよう、ModalShell (ConfirmDialog の土台) と同じく document.body へ portal で描画する。
// ブラウザ標準のメニューは呼び出し側が preventDefault で抑止する。
export function ContextMenu({ position, items, onClose }: Props) {
  const menuRef = useRef<HTMLDivElement>(null);
  const [placed, setPlaced] = useState<MenuPosition | null>(null);

  // サイズは描画後でないと分からないため、測ってから位置を補正する。
  // 補正前の位置が一瞬見えないよう、補正が済むまで visibility を隠す。
  useLayoutEffect(() => {
    const el = menuRef.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    setPlaced(
      clampMenuPosition(
        position,
        { width: rect.width, height: rect.height },
        { width: window.innerWidth, height: window.innerHeight },
      ),
    );
  }, [position, items.length]);

  // visibility: hidden の要素にはフォーカスできないので、位置が確定してから。
  const isPlaced = placed !== null;
  useEffect(() => {
    if (isPlaced) focusFirstItem(menuRef.current);
  }, [isPlaced]);

  useEffect(() => {
    function onMouseDown(e: MouseEvent) {
      if (e.target instanceof Node && menuRef.current?.contains(e.target)) return;
      onClose();
    }
    function onKeyDown(e: KeyboardEvent) {
      if (e.key === "Escape" || e.key === "Tab") {
        e.preventDefault();
        // ダイアログ内の入力欄から開いたとき、同じEscapeでダイアログまで
        // 閉じないよう、背後のdocumentのkeydownリスナーへ伝えない。
        e.stopPropagation();
        onClose();
        return;
      }
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        moveMenuFocus(menuRef.current, e.key === "ArrowDown" ? "next" : "prev");
      }
    }
    // 一覧など、どの要素のスクロールでも閉じる(capture でバブリングしない
    // scroll も拾う)。
    document.addEventListener("mousedown", onMouseDown, true);
    document.addEventListener("keydown", onKeyDown, true);
    document.addEventListener("scroll", onClose, true);
    document.addEventListener("wheel", onClose, true);
    window.addEventListener("blur", onClose);
    window.addEventListener("resize", onClose);
    return () => {
      document.removeEventListener("mousedown", onMouseDown, true);
      document.removeEventListener("keydown", onKeyDown, true);
      document.removeEventListener("scroll", onClose, true);
      document.removeEventListener("wheel", onClose, true);
      window.removeEventListener("blur", onClose);
      window.removeEventListener("resize", onClose);
    };
  }, [onClose]);

  const shown = placed ?? position;
  return createPortal(
    <div
      ref={menuRef}
      className="context-menu"
      role="menu"
      data-testid="context-menu"
      style={{
        left: shown.x,
        top: shown.y,
        visibility: placed ? "visible" : "hidden",
      }}
      onContextMenu={(e) => e.preventDefault()}
    >
      {items.map((item) => (
        <button
          key={item.label}
          type="button"
          role="menuitem"
          className={
            item.danger ? "context-menu-item context-menu-item--danger" : "context-menu-item"
          }
          disabled={item.disabled}
          aria-disabled={item.disabled || undefined}
          // 初期フォーカスが先頭項目に残ると、ホバー中の項目と二重に強調される。
          // 強調を常に1か所にするため、ホバーした項目へフォーカスを追従させる。
          // 無効な項目は focus() しても何も起きないので、フォーカスは動かさない。
          onMouseEnter={(e) => {
            if (!item.disabled) e.currentTarget.focus();
          }}
          onClick={() => {
            onClose();
            item.onSelect();
          }}
        >
          {item.label}
        </button>
      ))}
    </div>,
    document.body,
  );
}
