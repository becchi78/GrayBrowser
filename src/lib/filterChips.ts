// 絞り込み中バーに並べるチップ(選択中のフォルダ・タグ・★)を組み立てる純粋な
import { ratingLabel } from "./ratingBar.ts";

// 関数。表示と解除の対象を同じ場所で決めるため、コンポーネントから切り出した。

export type FilterChip =
  | { kind: "folder"; label: string }
  | { kind: "tag"; id: number; label: string }
  | { kind: "rating"; value: number; label: string };

export interface FilterSelection {
  folderPath: string | null;
  tagIds: readonly number[];
  ratings: readonly number[];
}

/** 何か1つでも絞り込みがあるか。バー全体の表示/非表示を決める。 */
export function hasActiveFilter({ folderPath, tagIds, ratings }: FilterSelection): boolean {
  return folderPath !== null || tagIds.length > 0 || ratings.length > 0;
}

/**
 * フォルダ → タグ(選択した順)→ ★(昇順)のチップ。名前が分からないタグ
 * (一覧の取得前、または削除済み)は、名前の無いチップを出さず省く。
 * バー自体は hasActiveFilter で出るので、「すべて解除」は使える。
 */
export function buildFilterChips(
  { folderPath, tagIds, ratings }: FilterSelection,
  tagNames: ReadonlyMap<number, string>,
): FilterChip[] {
  const chips: FilterChip[] = [];
  if (folderPath !== null) chips.push({ kind: "folder", label: folderPath });
  for (const id of tagIds) {
    const name = tagNames.get(id);
    if (name !== undefined) chips.push({ kind: "tag", id, label: name });
  }
  for (const value of [...ratings].sort((a, b) => a - b)) {
    chips.push({ kind: "rating", value, label: ratingLabel(value) });
  }
  return chips;
}

/** 3桁区切りの件数。 */
export function formatCount(n: number): string {
  return n.toLocaleString("ja-JP");
}
