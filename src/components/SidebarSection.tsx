import type { ReactNode } from "react";

interface Props {
  /** data-testid の接頭辞(`sidebar-section-<id>` / `-toggle` / `-body`)。 */
  id: string;
  title: string;
  collapsed: boolean;
  onToggle: () => void;
  children: ReactNode;
}

// 見出しのトグルボタン+本体。折りたたみ時は本体をマウントしない
// (中の部品が持つ取得処理・イベント購読を止めるため)。
export function SidebarSection({ id, title, collapsed, onToggle, children }: Props) {
  return (
    <section className="sidebar-section" data-testid={`sidebar-section-${id}`}>
      <button
        type="button"
        className="sidebar-section-toggle"
        data-testid={`sidebar-section-${id}-toggle`}
        aria-expanded={!collapsed}
        onClick={onToggle}
      >
        <span aria-hidden="true">{collapsed ? "▸" : "▾"}</span> {title}
      </button>
      {!collapsed && (
        <div className="sidebar-section-body" data-testid={`sidebar-section-${id}-body`}>
          {children}
        </div>
      )}
    </section>
  );
}
