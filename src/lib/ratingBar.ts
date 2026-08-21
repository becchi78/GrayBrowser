// RatingBar.tsx（JSXを含むコンポーネントファイル）から切り出した、JSX非依存の
// 純粋関数/定数群。src/lib/tagBarLayout.ts・src/lib/sidebarResize.ts と同じ
// パターン（コンポーネント側からimportして使う）。

/** `<select>`の選択肢一覧。空文字列は「すべて表示」(フィルタなし)を表す。 */
export const RATING_FILTER_OPTIONS: { value: string; label: string }[] = [
  { value: "", label: "すべて表示" },
  { value: "1", label: "★1以上" },
  { value: "2", label: "★2以上" },
  { value: "3", label: "★3以上" },
  { value: "4", label: "★4以上" },
  { value: "5", label: "★5以上" },
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
