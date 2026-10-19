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
 * タグ部品の並びを導出する。
 *
 * - `primary`: 選択中だが未常設のタグ(選択順=昇格) → 常設タグ(常設の順)。
 *   存在しないタグID(削除直後など)は除く。
 * - `others`: それ以外のタグ(`allTags`の順)。「その他のタグ」の展開先。
 *
 * 昇格用の専用stateを持たず毎回導出するので、選択を外せば自動的に昇格も消える。
 */
export function deriveTagLists<T extends { id: number }>(
  allTags: readonly T[],
  pinnedTagIds: readonly number[],
  selected: readonly number[],
): { primary: T[]; others: T[] } {
  const byId = new Map(allTags.map((tag) => [tag.id, tag]));
  const pinnedSet = new Set(pinnedTagIds);
  const primaryIds: number[] = [];
  const seen = new Set<number>();
  for (const id of [...selected.filter((id) => !pinnedSet.has(id)), ...pinnedTagIds]) {
    if (byId.has(id) && !seen.has(id)) {
      seen.add(id);
      primaryIds.push(id);
    }
  }
  return {
    primary: primaryIds.map((id) => byId.get(id) as T),
    others: allTags.filter((tag) => !seen.has(tag.id)),
  };
}
