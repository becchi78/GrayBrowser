import type { TagDto } from "../types.ts";

/**
 * 入力文字列に部分一致する既存タグだけに絞る。大文字小文字は区別しない。
 * 空入力は絞り込まず全件を返す(一覧から選ぶ操作を主にするため)。
 * 正規化(全角→半角など)の規則はバックエンドが持つので、ここでは表示用の
 * 絞り込みにとどめ、付与の可否は判断しない。
 */
export function filterTagCandidates(tags: TagDto[], input: string): TagDto[] {
  const needle = input.trim().toLowerCase();
  if (needle === "") return tags;
  return tags.filter((t) => t.name.toLowerCase().includes(needle));
}
