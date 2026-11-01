// LeftPane/TagBar から切り出した、JSX非依存の純粋関数群。
// src/lib/sidebarResize.ts と同じパターン(コンポーネント側からimportして使う)。

export type SidebarSectionId = "folders" | "rating" | "tags";

/** `true` = 折りたたみ。Rust側 `SidebarSectionsCollapsed` と同じ形。 */
export type SidebarSectionsCollapsed = Record<SidebarSectionId, boolean>;

export const DEFAULT_SIDEBAR_SECTIONS_COLLAPSED: SidebarSectionsCollapsed = {
  folders: false,
  rating: false,
  tags: false,
};

/** 指定セクションの折りたたみだけを反転した新しい状態を返す(元は変更しない)。 */
export function toggleSection(
  state: SidebarSectionsCollapsed,
  id: SidebarSectionId,
): SidebarSectionsCollapsed {
  return { ...state, [id]: !state[id] };
}
