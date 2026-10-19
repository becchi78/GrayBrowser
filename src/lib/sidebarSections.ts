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

/**
 * 左ペインのタグ一覧の並びを導出する: 常設タグ(常設の順) → それ以外(`allTags`の順)。
 * 存在しないタグID(削除直後など)と常設内の重複は除く。
 */
export function deriveTagLists<T extends { id: number }>(
  allTags: readonly T[],
  pinnedTagIds: readonly number[],
): T[] {
  const byId = new Map(allTags.map((tag) => [tag.id, tag]));
  const pinned: T[] = [];
  const seen = new Set<number>();
  for (const id of pinnedTagIds) {
    const tag = byId.get(id);
    if (tag && !seen.has(id)) {
      seen.add(id);
      pinned.push(tag);
    }
  }
  return [...pinned, ...allTags.filter((tag) => !seen.has(tag.id))];
}
