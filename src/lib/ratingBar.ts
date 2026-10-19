// RatingBar.tsx（JSXを含むコンポーネントファイル）から切り出した、JSX非依存の
// 純粋関数/定数群。src/lib/sidebarResize.ts と同じ
// パターン（コンポーネント側からimportして使う）。

/**
 * 絞り込みに使える評価の選択肢。ラベルは`StarRating`の入力用★列と同じ
 * 「★をN個並べる」表記に揃える。未評価(0)での抽出は提供しない。
 */
export const RATING_FILTER_OPTIONS: { value: number; label: string }[] = [
  { value: 1, label: "★" },
  { value: 2, label: "★★" },
  { value: 3, label: "★★★" },
  { value: 4, label: "★★★★" },
  { value: 5, label: "★★★★★" },
];

/**
 * 有効な評価の集合に対して`value`の有効/無効を切り替えた、新しい配列を返す
 * (入力は変更しない)。結果は常に昇順にそろえる: クリック順で配列の中身が
 * 変わると、同じ集合なのに再取得が走ってしまうため。
 */
export function toggleRating(ratings: readonly number[], value: number): number[] {
  const next = ratings.includes(value)
    ? ratings.filter((r) => r !== value)
    : [...ratings, value];
  return next.sort((a, b) => a - b);
}
