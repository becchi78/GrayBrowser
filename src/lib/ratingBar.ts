// RatingBar.tsx（JSXを含むコンポーネントファイル）から切り出した、JSX非依存の
// 純粋関数/定数群。src/lib/tagBarLayout.ts・src/lib/sidebarResize.ts と同じ
// パターン（コンポーネント側からimportして使う）。

/**
 * `<select>`の選択肢一覧。空文字列は「フィルタなし」を表し、ラベルは
 * `StarRating`の入力用★列と同じ「★をN個並べる」表記に揃える(「N以上」の
 * 文言を省き、★の個数自体がしきい値を表す)。
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
 * `<select>`のonChangeイベントで受け取る生の文字列値を、`minRating`
 * (number | null)に変換する。空文字列("すべて表示")はnull、それ以外は
 * Numberに変換する。
 */
export function parseRatingFilterValue(raw: string): number | null {
  if (raw === "") return null;
  return Number(raw);
}

// App.css`.rating-bar`の固定幅(8em = 128px、:rootのfont-size: 16px基準)に、
// `.header-row-filters`の`gap: 0.5em`(8px)を加えた値と一致させること --
// flexアイテム間のgapは`.tag-bar`自身の実際の描画幅からも同じだけ奪われるため、
// `.rating-bar`自体の幅だけを差し引くとその分ズレる。tagBarLayout.ts
// `computeVisibleChipCount`がTagBarの利用可能幅を計算する際、RatingBarが
// 同じ行に並ぶ分だけ差し引くために使う。
export const RATING_BAR_RESERVED_WIDTH_PX = 136;
