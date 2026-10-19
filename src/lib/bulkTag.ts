import type { TagDto } from "../types.ts";

export interface BulkTagCounts {
  newlyTagged: number;
  alreadyTagged: number;
  missing: number;
}

/**
 * 一括付与の結果を1行ずつの文に直す。0件の項目は出さない(全件新規のとき
 * に「既に付いていた 0 件」と並べても情報が増えないため)。
 */
export function bulkTagResultLines(tagName: string, counts: BulkTagCounts): string[] {
  const lines: string[] = [];
  if (counts.newlyTagged > 0) {
    lines.push(`タグ「${tagName}」を ${counts.newlyTagged} 件に付けました。`);
  }
  if (counts.alreadyTagged > 0) {
    lines.push(`${counts.alreadyTagged} 件は既に付いていました。`);
  }
  if (counts.missing > 0) {
    lines.push(`${counts.missing} 件は見つかりませんでした(削除または移動された可能性があります)。`);
  }
  if (lines.length === 0) {
    lines.push("対象の動画がありませんでした。");
  }
  return lines;
}

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
