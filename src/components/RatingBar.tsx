import { RATING_FILTER_OPTIONS, parseRatingFilterValue } from "../lib/ratingBar";

interface Props {
  /** `null` = "ALL"(評価による絞り込みなし)。 */
  value: number | null;
  onChange: (v: number | null) => void;
}

// タグ部品と並ぶ独立したフィルタ軸。タグ機構(TagBar/tags/video_tags)には一切
// 関与しない -- videos.ratingのみを対象に、選んだ評価と一致する動画だけを表示する単純なフィルタ。
export function RatingBar({ value, onChange }: Props) {
  const current = value === null ? "" : String(value);
  return (
    <div className="rating-bar" data-testid="rating-bar" role="group" aria-label="評価で絞り込み">
      {RATING_FILTER_OPTIONS.map((opt) => (
        <button
          key={opt.value}
          type="button"
          className={
            current === opt.value ? "rating-bar-item rating-bar-item--active" : "rating-bar-item"
          }
          data-testid="rating-bar-item"
          data-rating-value={opt.value}
          aria-pressed={current === opt.value}
          onClick={() => onChange(parseRatingFilterValue(opt.value))}
        >
          {opt.label}
        </button>
      ))}
    </div>
  );
}
