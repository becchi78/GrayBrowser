import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { clampMenuPosition, type MenuPosition } from "../lib/contextMenu";

export interface ContextMenuItem {
  label: string;
  danger?: boolean;
  onSelect: () => void;
}

interface Props {
  /** ポインタの位置(viewport 座標)。画面外にはみ出す場合は内側へ補正する。 */
  position: MenuPosition;
  items: ContextMenuItem[];
  onClose: () => void;
}

// 一覧のスクロール領域や重複パネルなど position / overflow の効く祖先の
// 影響を受けないよう、ConfirmDialog と同じく document.body へ portal で描画する。
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
    if (isPlaced) menuRef.current?.querySelector<HTMLElement>('[role="menuitem"]')?.focus();
  }, [isPlaced]);

  useEffect(() => {
    function onMouseDown(e: MouseEvent) {
      if (e.target instanceof Node && menuRef.current?.contains(e.target)) return;
      onClose();
    }
    function onKeyDown(e: KeyboardEvent) {
      if (e.key === "Escape" || e.key === "Tab") {
        e.preventDefault();
        onClose();
        return;
      }
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        const entries = Array.from(
          menuRef.current?.querySelectorAll<HTMLElement>('[role="menuitem"]') ?? [],
        );
        if (entries.length === 0) return;
        const current = entries.indexOf(document.activeElement as HTMLElement);
        const step = e.key === "ArrowDown" ? 1 : -1;
        entries[(current + step + entries.length) % entries.length].focus();
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
