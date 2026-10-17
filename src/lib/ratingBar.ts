// RatingBar.tsx（JSXを含むコンポーネントファイル）から切り出した、JSX非依存の
// 純粋関数/定数群。src/lib/tagBarLayout.ts・src/lib/sidebarResize.ts と同じ
// パターン（コンポーネント側からimportして使う）。

/**
 * `<select>`の選択肢一覧。空文字列は「フィルタなし」を表し、ラベルは
 * `StarRating`の入力用★列と同じ「★をN個並べる」表記に揃える(「N以上」の
 * 文言を省き、★の個数自体が絞り込む評価(その評価と一致するもののみ)を表す)。
 */
export const RATING_FILTER_OPTIONS: { value: string; label: string }[] = [
  { value: "", label: "ALL" },
  { value: "1", label: "★" },
  { value: "2", label: "★★" },
  { value: "3", label: "★★★" },
  { value: "4", label: "★★★★" },
  { value: "5", label: "★★★★★" },
];

/**
 * `<select>`のonChangeイベントで受け取る生の文字列値を、`rating`
 * (number | null)に変換する。空文字列("すべて表示")はnull、それ以外は
 * Numberに変換する。
 */
export function parseRatingFilterValue(raw: string): number | null {
  if (raw === "") return null;
  return Number(raw);
}
